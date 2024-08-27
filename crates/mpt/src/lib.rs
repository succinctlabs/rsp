use std::collections::{btree_map::Entry, BTreeMap, HashSet};

use alloy_primitives::Bytes;
use alloy_rlp::{Decodable, Encodable};
use eyre::Ok;
use itertools::Either;
use reth_execution_types::ExecutionOutcome;
use reth_primitives::{Address, B256};
use reth_trie::{
    AccountProof, HashBuilder, HashedPostState, HashedStorage, Nibbles, TrieAccount, TrieNode,
    CHILD_INDEX_RANGE, EMPTY_ROOT_HASH,
};
use revm_primitives::{keccak256, HashMap};
use rsp_primitives::storage::ExtDatabaseRef;

#[cfg(feature = "preimage_context")]
use rsp_primitives::storage::PreimageContext;

/// Additional context for preimage recovery when calculating trie root. `Some` when calculating
/// storage trie root and `None` when calculating state trie root.
#[cfg(feature = "preimage_context")]
type RootContext = Option<Address>;

/// No additional context is needed since the `preimage_context` feature is disabled.
#[cfg(not(feature = "preimage_context"))]
type RootContext = ();

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
            #[cfg(feature = "preimage_context")]
            let context = Some(address);
            #[cfg(not(feature = "preimage_context"))]
            let context = ();

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
                context,
            )?
        };
        storage_roots.insert(hashed_address, root);
    }

    #[cfg(feature = "preimage_context")]
    let context = None;
    #[cfg(not(feature = "preimage_context"))]
    let context = ();

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
        context,
    )
}

/// Given a list of Merkle-Patricia proofs, compute the root of the trie.
fn compute_root_from_proofs<DB>(
    items: impl IntoIterator<Item = (Nibbles, Option<Vec<u8>>, Vec<Bytes>)>,
    db: &DB,
    #[allow(unused)] root_context: RootContext,
) -> eyre::Result<B256>
where
    DB: ExtDatabaseRef<Error: std::fmt::Debug>,
{
    let mut trie_nodes = BTreeMap::default();
    let mut ignored_keys = HashSet::<Nibbles>::default();

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
                    if next_path == key {
                        if value.is_none() {
                            // The proof points to the node of interest, meaning the node previously
                            // exists. The node does not exist now, so the parent pointing to this
                            // child needs to be eliminated too.

                            // Recover the path before the extensions. We either have to clone
                            // before the extension or recover here. Recovering here is probably
                            // more efficient as long as deletion is not the majority of the
                            // updates.
                            ignored_keys
                                .insert(next_path.slice(0..(next_path.len() - leaf.key.len())));
                        }
                    } else {
                        // The proof points to a neighbour. This happens when proving the previous
                        // absence of the node of interest.
                        //
                        // We insert this neighbour node only if it's vacant to avoid overwriting
                        // it when the neighbour node itself is being updated.
                        if let Entry::Vacant(entry) = trie_nodes.entry(next_path.clone()) {
                            entry.insert(Either::Right(leaf.value.clone()));
                        }
                    }
                }
            };
            path = next_path;
        }

        if let Some(value) = value {
            // This overwrites any value that might have been inserted during proof walking, which
            // can happen when an immediate upper neighbour is inserted where the already inserted
            // value would be outdated.
            trie_nodes.insert(key, Either::Right(value));
        } else {
            // This is a node deletion. If this key is not ignored then an insertion of an immediate
            // upper neighbour would result in this node being added (and thus treated as not
            // deleted) as part of the proof walking process.
            ignored_keys.insert(key);
        }
    }

    // Ignore branch child hashes in the path of leaves or lower child hashes.
    let mut keys = trie_nodes.keys().peekable();
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
                let has_neighbour = (!hash_builder.key.is_empty() &&
                    hash_builder.key.starts_with(&parent_branch_path)) ||
                    trie_nodes
                        .peek()
                        .map_or(false, |next| next.0.starts_with(&parent_branch_path));

                if has_neighbour {
                    hash_builder.add_branch(path, branch_hash, false);
                } else {
                    // Parent was a branch node but now all but one children are gone. We
                    // technically have to modify this branch node, but the `alloy-trie` hash
                    // builder handles this automatically when supplying child nodes.

                    #[cfg(feature = "preimage_context")]
                    let preimage = db
                        .trie_node_ref_with_context(
                            branch_hash,
                            PreimageContext { address: &root_context, branch_path: &path },
                        )
                        .unwrap();
                    #[cfg(not(feature = "preimage_context"))]
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

#[cfg(test)]
mod tests {
    use super::*;

    use alloy_trie::proof::ProofRetainer;
    use hex_literal::hex;

    /// Leaf node A:
    ///
    /// e1 => list len = 33
    ///    3a => odd leaf with path `a`
    ///    9f => string len = 31
    ///       9e => string len = 30
    ///          888888888888888888888888888888888888888888888888888888888888 => value
    ///
    /// Flattened:
    /// e13a9f9e888888888888888888888888888888888888888888888888888888888888
    ///
    /// Trie node hash:
    /// c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421
    const LEAF_A: Bytes = Bytes::from_static(&hex!(
        "e13a9f9e888888888888888888888888888888888888888888888888888888888888"
    ));

    struct TestTrieDb {
        preimages: Vec<Bytes>,
    }

    impl TestTrieDb {
        fn new() -> Self {
            Self { preimages: vec![LEAF_A] }
        }
    }

    impl ExtDatabaseRef for TestTrieDb {
        type Error = std::convert::Infallible;

        fn trie_node_ref(&self, hash: B256) -> std::result::Result<Bytes, Self::Error> {
            for preimage in self.preimages.iter() {
                if keccak256(preimage) == hash {
                    return std::result::Result::Ok(preimage.to_owned());
                }
            }

            panic!("missing preimage for test")
        }

        fn trie_node_ref_with_context(
            &self,
            hash: B256,
            _context: PreimageContext<'_>,
        ) -> Result<Bytes, Self::Error> {
            self.trie_node_ref(hash)
        }
    }

    #[test]
    fn test_delete_single_leaf() {
        // Trie before with nodes
        //
        // - `1a`: 888888888888888888888888888888888888888888888888888888888888
        // - `2a`: 888888888888888888888888888888888888888888888888888888888888
        // - `3a`: 888888888888888888888888888888888888888888888888888888888888
        //
        // Root:
        //
        // f8 => list len of len = 1
        //    71 => list len = 113
        //       80
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //
        // Flattened:
        // f87180a0c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c7
        // 2d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c72d0c79d673ad5a
        // cb10a951f0e82cf2e461b98c4e5afb0348ccab5bb42180808080808080808080808080
        //
        // Root hash
        // 929a169d86a02de55457b8928bd3cdae55b24fe2771f7a3edaa992c0500c4427

        // Deleting node `2a`:
        //
        // - `1a`: 888888888888888888888888888888888888888888888888888888888888
        // - `3a`: 888888888888888888888888888888888888888888888888888888888888
        //
        // New root:
        //
        // f8 => list len of len = 1
        //    51 => list len = 81
        //       80
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       80
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //
        // Flattened:
        // f85180a0c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb42180a0c2c2
        // c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb42180808080808080808080
        // 808080
        //
        // Root hash
        // ff07cbbe26d25f65cf2ff08dc127e71b8cb238bee5da9df515422ff7eaa8d67e

        let root = compute_root_from_proofs(
            [(
                Nibbles::from_nibbles([0x2, 0xa]),
                None,
                vec![
                    Bytes::from_static(&hex!(
                        "\
f87180a0c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c7\
2d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c72d0c79d673ad5a\
cb10a951f0e82cf2e461b98c4e5afb0348ccab5bb42180808080808080808080808080"
                    )),
                    LEAF_A,
                ],
            )],
            &TestTrieDb::new(),
            None,
        )
        .unwrap();

        assert_eq!(root, hex!("ff07cbbe26d25f65cf2ff08dc127e71b8cb238bee5da9df515422ff7eaa8d67e"));
    }

    #[test]
    fn test_delete_multiple_leaves() {
        // Trie before with nodes
        //
        // - `1a`: 888888888888888888888888888888888888888888888888888888888888
        // - `2a`: 888888888888888888888888888888888888888888888888888888888888
        // - `3a`: 888888888888888888888888888888888888888888888888888888888888
        // - `4a`: 888888888888888888888888888888888888888888888888888888888888
        //
        // Root:
        //
        // f8 => list len of len = 1
        //    91 => list len = 145
        //       80
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //
        // Flattened:
        // f89180a0c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c7
        // 2d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c72d0c79d673ad5a
        // cb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c72d0c79d673ad5acb10a951f0e82c
        // f2e461b98c4e5afb0348ccab5bb421808080808080808080808080
        //
        // Root hash
        // d34c1443edf7e282fcfd056db2ec24bcaf797dc3a039e0628473b069a2e8b1be

        // Deleting node `2a` and `3a`:
        //
        // - `1a`: 888888888888888888888888888888888888888888888888888888888888
        //
        // New root:
        //
        // f8 => list len of len = 1
        //    51 => list len = 81
        //       80
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       80
        //       80
        //       a0 => branch hash
        //          c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421 => leaf node A
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //       80
        //
        // Flattened:
        // f85180a0c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb4218080a0c2
        // c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421808080808080808080
        // 808080
        //
        // Root hash
        // 4a2aa1a2188e9bf279d51729b0c5789e4f0605c85752f9ca47760fcbe0f80244

        let root = compute_root_from_proofs(
            [
                (
                    Nibbles::from_nibbles([0x2, 0xa]),
                    None,
                    vec![
                        Bytes::from_static(&hex!(
                            "\
f89180a0c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c7\
2d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c72d0c79d673ad5a\
cb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c72d0c79d673ad5acb10a951f0e82c\
f2e461b98c4e5afb0348ccab5bb421808080808080808080808080"
                        )),
                        LEAF_A,
                    ],
                ),
                (
                    Nibbles::from_nibbles([0x3, 0xa]),
                    None,
                    vec![
                        Bytes::from_static(&hex!(
                            "\
f89180a0c2c2c72d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c7\
2d0c79d673ad5acb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c72d0c79d673ad5a\
cb10a951f0e82cf2e461b98c4e5afb0348ccab5bb421a0c2c2c72d0c79d673ad5acb10a951f0e82c\
f2e461b98c4e5afb0348ccab5bb421808080808080808080808080"
                        )),
                        LEAF_A,
                    ],
                ),
            ],
            &TestTrieDb::new(),
            None,
        )
        .unwrap();

        assert_eq!(root, hex!("4a2aa1a2188e9bf279d51729b0c5789e4f0605c85752f9ca47760fcbe0f80244"));
    }

    #[test]
    fn test_insert_with_updated_neighbour() {
        let value_1 = hex!("9e888888888888888888888888888888888888888888888888888888888888");
        let value_2 = hex!("9e999999999999999999999999999999999999999999999999999999999999");

        // Trie before as a branch with 2 nodes:
        //
        // - `11a`: 888888888888888888888888888888888888888888888888888888888888
        // - `2a`: 888888888888888888888888888888888888888888888888888888888888

        let mut hash_builder =
            HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![
                Nibbles::from_nibbles([0x1, 0x1, 0xa]),
                Nibbles::from_nibbles([0x1, 0x1, 0xb]),
            ]));
        hash_builder.add_leaf(Nibbles::from_nibbles([0x1, 0x1, 0xa]), &value_1);
        hash_builder.add_leaf(Nibbles::from_nibbles([0x2, 0xa]), &value_1);

        hash_builder.root();
        let proofs = hash_builder.take_proofs();

        // Trie after updating `11a` and inserting `11b`:
        //
        // - `11a`: 999999999999999999999999999999999999999999999999999999999999
        // - `11b`: 888888888888888888888888888888888888888888888888888888888888
        // - `2a`: 888888888888888888888888888888888888888888888888888888888888
        //
        // Root branch child slot 1 turns from a leaf to another branch.

        let mut hash_builder = HashBuilder::default();
        hash_builder.add_leaf(Nibbles::from_nibbles([0x1, 0x1, 0xa]), &value_2);
        hash_builder.add_leaf(Nibbles::from_nibbles([0x1, 0x1, 0xb]), &value_1);
        hash_builder.add_leaf(Nibbles::from_nibbles([0x2, 0xa]), &value_1);

        let root = compute_root_from_proofs(
            [
                (
                    Nibbles::from_nibbles([0x1, 0x1, 0xa]),
                    Some(
                        hex!("9e999999999999999999999999999999999999999999999999999999999999")
                            .to_vec(),
                    ),
                    vec![
                        proofs.get(&Nibbles::default()).unwrap().to_owned(),
                        proofs.get(&Nibbles::from_nibbles([0x1])).unwrap().to_owned(),
                    ],
                ),
                (
                    Nibbles::from_nibbles([0x1, 0x1, 0xb]),
                    Some(
                        hex!("9e888888888888888888888888888888888888888888888888888888888888")
                            .to_vec(),
                    ),
                    vec![
                        proofs.get(&Nibbles::default()).unwrap().to_owned(),
                        proofs.get(&Nibbles::from_nibbles([0x1])).unwrap().to_owned(),
                    ],
                ),
            ],
            &TestTrieDb::new(),
            None,
        )
        .unwrap();

        assert_eq!(root, hash_builder.root());
    }

    #[test]
    fn test_insert_with_deleted_neighbour() {
        let value = hex!("9e888888888888888888888888888888888888888888888888888888888888");

        // Trie before as a branch with 2 nodes:
        //
        // - `11a`: 888888888888888888888888888888888888888888888888888888888888
        // - `2a`: 888888888888888888888888888888888888888888888888888888888888

        let mut hash_builder =
            HashBuilder::default().with_proof_retainer(ProofRetainer::new(vec![
                Nibbles::from_nibbles([0x1, 0x1, 0xa]),
                Nibbles::from_nibbles([0x1, 0x1, 0xb]),
            ]));
        hash_builder.add_leaf(Nibbles::from_nibbles([0x1, 0x1, 0xa]), &value);
        hash_builder.add_leaf(Nibbles::from_nibbles([0x2, 0xa]), &value);

        hash_builder.root();
        let proofs = hash_builder.take_proofs();

        // Trie after deleting `11a` and inserting `11b`:
        //
        // - `11b`: 888888888888888888888888888888888888888888888888888888888888
        // - `2a`: 888888888888888888888888888888888888888888888888888888888888
        //
        // Root branch child slot 1 turns from a leaf to another branch.

        let mut hash_builder = HashBuilder::default();
        hash_builder.add_leaf(Nibbles::from_nibbles([0x1, 0x1, 0xb]), &value);
        hash_builder.add_leaf(Nibbles::from_nibbles([0x2, 0xa]), &value);

        let root = compute_root_from_proofs(
            [
                (
                    Nibbles::from_nibbles([0x1, 0x1, 0xa]),
                    None,
                    vec![
                        proofs.get(&Nibbles::default()).unwrap().to_owned(),
                        proofs.get(&Nibbles::from_nibbles([0x1])).unwrap().to_owned(),
                    ],
                ),
                (
                    Nibbles::from_nibbles([0x1, 0x1, 0xb]),
                    Some(
                        hex!("9e888888888888888888888888888888888888888888888888888888888888")
                            .to_vec(),
                    ),
                    vec![
                        proofs.get(&Nibbles::default()).unwrap().to_owned(),
                        proofs.get(&Nibbles::from_nibbles([0x1])).unwrap().to_owned(),
                    ],
                ),
            ],
            &TestTrieDb::new(),
            None,
        )
        .unwrap();

        assert_eq!(root, hash_builder.root());
    }

    #[test]
    fn test_only_root_node_left() {
        let value = hex!("9e888888888888888888888888888888888888888888888888888888888888");

        // Trie before as a branch with 2 nodes:
        //
        // - `1a`: 888888888888888888888888888888888888888888888888888888888888
        // - `2a`: 888888888888888888888888888888888888888888888888888888888888

        let mut hash_builder = HashBuilder::default()
            .with_proof_retainer(ProofRetainer::new(vec![Nibbles::from_nibbles([0x1, 0xa])]));
        hash_builder.add_leaf(Nibbles::from_nibbles([0x1, 0xa]), &value);
        hash_builder.add_leaf(Nibbles::from_nibbles([0x2, 0xa]), &value);

        hash_builder.root();
        let proofs = hash_builder.take_proofs();

        dbg!(&proofs);

        // Trie after deleting `1a`:
        //
        // - `2a`: 888888888888888888888888888888888888888888888888888888888888
        //
        // Root branch child slot 1 turns from a leaf to another branch.

        let mut hash_builder = HashBuilder::default();
        hash_builder.add_leaf(Nibbles::from_nibbles([0x2, 0xa]), &value);

        let root = compute_root_from_proofs(
            [(
                Nibbles::from_nibbles([0x1, 0xa]),
                None,
                vec![
                    proofs.get(&Nibbles::default()).unwrap().to_owned(),
                    proofs.get(&Nibbles::from_nibbles([0x1])).unwrap().to_owned(),
                ],
            )],
            &TestTrieDb::new(),
            None,
        )
        .unwrap();

        assert_eq!(root, hash_builder.root());
    }
}
