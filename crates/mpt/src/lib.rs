use std::collections::{BTreeMap, HashSet};

use alloy_primitives::Bytes;
use alloy_rlp::{Decodable, Encodable};
use alloy_rpc_types::EIP1186AccountProofResponse;
use eyre::Ok;
use itertools::Either;
use reth_execution_types::ExecutionOutcome;
use reth_primitives::{Address, B256};
use reth_trie::{
    nodes::{TrieNode, CHILD_INDEX_RANGE},
    HashBuilder, HashedPostState, HashedStorage, Nibbles, TrieAccount, EMPTY_ROOT_HASH,
};
use revm_primitives::{keccak256, HashMap};

/// Computes the state root of a block's Merkle Patricia Trie given an [ExecutionOutcome] and a list
/// of [EIP1186AccountProofResponse] storage proofs.
pub fn compute_state_root(
    execution_outcome: &ExecutionOutcome,
    storage_proofs: &[EIP1186AccountProofResponse],
) -> eyre::Result<B256> {
    // Reconstruct prefix sets manually to record pre-images for subsequent lookups.
    let mut hashed_state = HashedPostState::default();
    let mut account_reverse_lookup = HashMap::<B256, Address>::default();
    let mut storage_reverse_lookup = HashMap::<B256, B256>::default();
    for (address, account) in execution_outcome.bundle_accounts_iter() {
        let hashed_address = keccak256(address);
        account_reverse_lookup.insert(hashed_address, address);
        hashed_state.accounts.insert(hashed_address, account.info.clone().map(Into::into));

        let mut storage_keys = Vec::new();
        let mut hashed_storage = HashedStorage::new(account.status.was_destroyed());
        for (key, value) in &account.storage {
            let slot = B256::new(key.to_be_bytes());
            let hashed_slot = keccak256(slot);
            storage_keys.push(slot);
            storage_reverse_lookup.insert(hashed_slot, slot);
            hashed_storage.storage.insert(hashed_slot, value.present_value);
        }

        hashed_state.storages.insert(hashed_address, hashed_storage);
    }

    // Compute the storage roots for each account.
    let mut storage_roots = HashMap::<B256, B256>::default();
    let prefix_sets = hashed_state.construct_prefix_sets();
    for account_nibbles in prefix_sets.account_prefix_set.keys.iter() {
        let hashed_address = B256::from_slice(&account_nibbles.pack());
        let address = *account_reverse_lookup.get(&hashed_address).unwrap();
        let storage_prefix_sets =
            prefix_sets.storage_prefix_sets.get(&hashed_address).cloned().unwrap_or_default();

        let proof = storage_proofs.iter().find(|x| x.address == address).unwrap();
        let root = if proof.storage_proof.is_empty() {
            proof.storage_hash
        } else {
            compute_root_from_proofs(storage_prefix_sets.keys.iter().map(|storage_nibbles| {
                let hashed_slot = B256::from_slice(&storage_nibbles.pack());
                let slot = storage_reverse_lookup.get(&hashed_slot).unwrap();
                let storage_proof = proof.storage_proof.iter().find(|x| &x.key.0 == slot).unwrap();
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
            }))?
        };
        storage_roots.insert(hashed_address, root);
    }

    // Compute the state root of the entire trie.
    let mut rlp_buf = Vec::with_capacity(128);
    compute_root_from_proofs(prefix_sets.account_prefix_set.keys.iter().map(|account_nibbles| {
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
        (account_nibbles.clone(), encoded, proof.account_proof.clone())
    }))
}

/// Given a list of Merkle-Patricia proofs, compute the root of the trie.
fn compute_root_from_proofs(
    items: impl IntoIterator<Item = (Nibbles, Option<Vec<u8>>, Vec<Bytes>)>,
) -> eyre::Result<B256> {
    let mut trie_nodes = BTreeMap::default();

    for (key, value, proof) in items {
        let mut path = Nibbles::default();
        for encoded in proof {
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
                                trie_nodes.insert(
                                    branch_child_path,
                                    Either::Left(B256::from_slice(&branch.stack[stack_ptr][1..])),
                                );
                            }
                            stack_ptr += 1;
                        }
                    }
                }
                TrieNode::Extension(extension) => {
                    next_path.extend_from_slice(&extension.key);
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
    while let Some((path, value)) = trie_nodes.next() {
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
                    // parent is a branch node that needs to be turned into extension
                    todo!()
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
