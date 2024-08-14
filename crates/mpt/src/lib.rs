use std::collections::{BTreeMap, HashSet};

use alloy_primitives::Bytes;
use alloy_rlp::{Decodable, Encodable};
use eyre::Ok;
use itertools::Either;
use reth_execution_types::ExecutionOutcome;
use reth_primitives::{Address, B256};
use reth_trie::{
    nodes::{TrieNode, CHILD_INDEX_RANGE},
    AccountProof, HashBuilder, HashedPostState, HashedStorage, Nibbles, TrieAccount,
    EMPTY_ROOT_HASH,
};
use revm_primitives::{keccak256, HashMap};
use rsp_primitives::storage::ExtDatabaseRef;

/// Computes the state root of a block's Merkle Patricia Trie given an [ExecutionOutcome] and a list
/// of [EIP1186AccountProofResponse] storage proofs.
pub fn compute_state_root<DB>(
    execution_outcome: &ExecutionOutcome,
    storage_proofs: &[AccountProof],
    db: &DB,
) -> eyre::Result<B256>
where
    DB: ExtDatabaseRef<Error: std::fmt::Debug>,
{
    // Reconstruct prefix sets manually to record pre-images for subsequent lookups.
    let mut hashed_state = HashedPostState::default();
    let mut account_reverse_lookup = HashMap::<B256, Address>::default();
    let mut storage_reverse_lookup = HashMap::<B256, B256>::default();
    for (address, account) in execution_outcome.bundle_accounts_iter() {
        let hashed_address = keccak256(address);
        account_reverse_lookup.insert(hashed_address, address);
        hashed_state.accounts.insert(hashed_address, account.info.clone().map(Into::into));

        let mut hashed_storage = HashedStorage::new(account.status.was_destroyed());
        for (key, value) in &account.storage {
            let slot = B256::new(key.to_be_bytes());
            let hashed_slot = keccak256(slot);
            storage_reverse_lookup.insert(hashed_slot, slot);
            hashed_storage.storage.insert(hashed_slot, value.present_value);
        }

        hashed_state.storages.insert(hashed_address, hashed_storage);
    }

    // Compute the storage roots for each account.
    let mut storage_roots = HashMap::<B256, B256>::default();
    let prefix_sets = hashed_state.construct_prefix_sets();
    let account_prefix_set = prefix_sets.account_prefix_set.freeze();
    for account_nibbles in account_prefix_set.iter() {
        let hashed_address = B256::from_slice(&account_nibbles.pack());
        let address = *account_reverse_lookup.get(&hashed_address).unwrap();
        let storage_prefix_sets =
            prefix_sets.storage_prefix_sets.get(&hashed_address).cloned().unwrap_or_default();

        let proof = storage_proofs.iter().find(|x| x.address == address).unwrap();
        let root = if proof.storage_proofs.is_empty() {
            proof.storage_root
        } else {
            compute_root_from_proofs(
                storage_prefix_sets.freeze().iter().map(|storage_nibbles| {
                    let hashed_slot = B256::from_slice(&storage_nibbles.pack());
                    let slot = storage_reverse_lookup.get(&hashed_slot).unwrap();
                    let storage_proof =
                        proof.storage_proofs.iter().find(|x| x.key.0 == slot).unwrap();
                    let encoded = Some(
                        hashed_state
                            .storages
                            .get(&hashed_address)
                            .and_then(|s| s.storage.get(&hashed_slot).cloned())
                            .unwrap_or_default(),
                    )
                    .filter(|v| !v.is_zero())
                    .map(|v| alloy_rlp::encode_fixed_size(&v).to_vec());
                    (storage_nibbles.clone(), encoded, storage_proof.proof.clone())
                }),
                db,
            )?
        };
        storage_roots.insert(hashed_address, root);
    }

    // Compute the state root of the entire trie.
    let mut rlp_buf = Vec::with_capacity(128);
    compute_root_from_proofs(
        account_prefix_set.iter().map(|account_nibbles| {
            let hashed_address = B256::from_slice(&account_nibbles.pack());
            let address = *account_reverse_lookup.get(&hashed_address).unwrap();
            let proof = storage_proofs.iter().find(|x| x.address == address).unwrap();

            let storage_root = *storage_roots.get(&hashed_address).unwrap();

            let account = hashed_state.accounts.get(&hashed_address).unwrap().unwrap_or_default();
            let encoded = if account.is_empty() && storage_root == EMPTY_ROOT_HASH {
                None
            } else {
                rlp_buf.clear();
                TrieAccount::from((account, storage_root)).encode(&mut rlp_buf);
                Some(rlp_buf.clone())
            };
            (account_nibbles.clone(), encoded, proof.proof.clone())
        }),
        db,
    )
}

/// Given a list of Merkle-Patricia proofs, compute the root of the trie.
fn compute_root_from_proofs<DB>(
    items: impl IntoIterator<Item = (Nibbles, Option<Vec<u8>>, Vec<Bytes>)>,
    db: &DB,
) -> eyre::Result<B256>
where
    DB: ExtDatabaseRef<Error: std::fmt::Debug>,
{
    let mut trie_nodes = BTreeMap::default();

    for (key, value, proof) in items {
        let mut path = Nibbles::default();
        let mut proof_iter = proof.iter().peekable();

        while let Some(encoded) = proof_iter.next() {
            let mut next_path = path.clone();
            match TrieNode::decode(&mut &encoded[..])? {
                TrieNode::Branch(branch) => {
                    next_path.push(key[path.len()]);
                    let mut stack_ptr = branch.as_ref().first_child_index();
                    for index in CHILD_INDEX_RANGE {
                        let mut branch_child_path = path.clone();
                        branch_child_path.push(index);

                        if branch.state_mask.is_bit_set(index) {
                            if !key.starts_with(&branch_child_path) {
                                let child = &branch.stack[stack_ptr];
                                if child.len() == B256::len_bytes() + 1 {
                                    // The child node is referred to by hash.
                                    trie_nodes.insert(
                                        branch_child_path,
                                        Either::Left(B256::from_slice(&child[1..])),
                                    );
                                } else {
                                    // The child node is encoded in-place. This can happen when the
                                    // encoded child itself is shorter than 32 bytes:
                                    //
                                    // https://github.com/ethereum/ethereum-org-website/blob/6eed7bcfd708ca605447dd9b8fde8f74cfcaf8d9/public/content/developers/docs/data-structures-and-encoding/patricia-merkle-trie/index.md?plain=1#L186
                                    if let TrieNode::Leaf(child_leaf) =
                                        TrieNode::decode(&mut &child[..])?
                                    {
                                        branch_child_path.extend_from_slice(&child_leaf.key);
                                        trie_nodes.insert(
                                            branch_child_path,
                                            Either::Right(child_leaf.value),
                                        );
                                    } else {
                                        // Same as the case for an extension node's child below,
                                        // this is possible in theory but extremely unlikely (even
                                        // more unlikely than the extension node's case as the node
                                        // header takes up extra space), making it impractical to
                                        // find proper test cases. It's better to be left
                                        // unimplemented.
                                        unimplemented!(
                                            "branch child is a non-leaf node encoded in place"
                                        );
                                    }
                                }
                            }
                            stack_ptr += 1;
                        }
                    }
                }
                TrieNode::Extension(extension) => {
                    next_path.extend_from_slice(&extension.key);

                    // Add the extended branch node if this is the last proof item. This can happen
                    // when proving the previous absence of a new node that shares the prefix with
                    // the extension node.
                    if proof_iter.peek().is_none() {
                        let child = &extension.child;
                        if child.len() == B256::len_bytes() + 1 {
                            // The extension child is referenced by hash.
                            trie_nodes.insert(
                                next_path.clone(),
                                Either::Left(B256::from_slice(&child[1..])),
                            );
                        } else {
                            // An extension's child can only be a branch. Since here it's also not a
                            // hash, it can only be a branch node encoded in place. This could
                            // happen in theory when two leaf nodes share a very long common prefix
                            // and both have very short values.
                            //
                            // In practice, since key paths are Keccak hashes, it's extremely
                            // difficult to get two slots like this for testing. Since this cannot
                            // be properly tested, it's more preferable to leave it unimplemented to
                            // be alerted when this is hit (which is extremely unlikely).
                            //
                            // Using `unimplemented!` instead of `todo!` because of this.
                            //
                            // To support this, the underlying `alloy-trie` crate (which is
                            // currently faulty for not supported in-place encoded nodes) must first
                            // be patched to support adding in-place nodes to the hash builder.
                            // Relevant PR highlighting the issue:
                            //
                            // https://github.com/alloy-rs/trie/pull/27
                            unimplemented!("extension child is a branch node encoded in place")
                        }
                    }
                }
                TrieNode::Leaf(leaf) => {
                    next_path.extend_from_slice(&leaf.key);
                    if next_path != key {
                        trie_nodes.insert(next_path.clone(), Either::Right(leaf.value.clone()));
                    }
                }
            };
            path = next_path;
        }

        if let Some(value) = value {
            trie_nodes.insert(key, Either::Right(value));
        }
    }

    // Ignore branch child hashes in the path of leaves or lower child hashes.
    let mut keys = trie_nodes.keys().peekable();
    let mut ignored_keys = HashSet::<Nibbles>::default();
    while let Some(key) = keys.next() {
        if keys.peek().map_or(false, |next| next.starts_with(key)) {
            ignored_keys.insert(key.clone());
        }
    }

    // Build the hash tree.
    let mut hash_builder = HashBuilder::default();
    let mut trie_nodes =
        trie_nodes.into_iter().filter(|(path, _)| !ignored_keys.contains(path)).peekable();
    while let Some((mut path, value)) = trie_nodes.next() {
        match value {
            Either::Left(branch_hash) => {
                let parent_branch_path = path.slice(..path.len() - 1);
                if hash_builder.key.starts_with(&parent_branch_path) ||
                    trie_nodes
                        .peek()
                        .map_or(false, |next| next.0.starts_with(&parent_branch_path))
                {
                    hash_builder.add_branch(path, branch_hash, false);
                } else {
                    // Parent was a branch node but now all but one children are gone. We
                    // technically have to modify this branch node, but the `alloy-trie` hash
                    // builder handles this automatically when supplying child nodes.

                    let preimage = db.trie_node_ref(branch_hash).unwrap();
                    match TrieNode::decode(&mut &preimage[..]).unwrap() {
                        TrieNode::Branch(_) => {
                            // This node is a branch node that's referenced by hash. There's no need
                            // to handle the content as the node itself is unchanged.
                            hash_builder.add_branch(path, branch_hash, false);
                        }
                        TrieNode::Extension(extension) => {
                            // This node is an extension node. Simply prepend the leaf node's key
                            // with the original branch index. `alloy-trie` automatically handles
                            // this so we only have to reconstruct the full key path.
                            path.extend_from_slice(&extension.key);

                            // In theory, it's possible that this extension node's child branch is
                            // encoded in-place, though it should be extremely rare, as for that to
                            // happen, at least 2 storage nodes must share a very long prefix, which
                            // is very unlikely to happen given that they're hashes.
                            //
                            // Moreover, `alloy-trie` currently does not offer an API for this rare
                            // case anyway. See relevant (but not directly related) PR:
                            //
                            // https://github.com/alloy-rs/trie/pull/27
                            if extension.child.len() == B256::len_bytes() + 1 {
                                hash_builder.add_branch(
                                    path,
                                    B256::from_slice(&extension.child[1..]),
                                    false,
                                );
                            } else {
                                todo!("handle in-place extension child")
                            }
                        }
                        TrieNode::Leaf(leaf) => {
                            // Same as the extension node's case: we only have to reconstruct the
                            // full path.
                            path.extend_from_slice(&leaf.key);
                            hash_builder.add_leaf(path, &leaf.value);
                        }
                    }
                }
            }
            Either::Right(leaf_value) => {
                hash_builder.add_leaf(path, &leaf_value);
            }
        }
    }
    let root = hash_builder.root();
    Ok(root)
}
