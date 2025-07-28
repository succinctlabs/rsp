use alloy_primitives::{keccak256, map::HashMap, B256};
use alloy_rlp::Decodable;
use alloy_rpc_types_debug::ExecutionWitness;
use reth_trie::TrieAccount;

use crate::mpt::{resolve_nodes, MptNode, MptNodeData, MptNodeReference};

// Builds tries from the witness state.
//
// NOTE: This method should be called outside zkVM! In general you construct tries, then
// validate them inside zkVM.
pub(crate) fn build_validated_tries(
    witness: &ExecutionWitness,
    pre_state_root: B256,
) -> Result<(MptNode, HashMap<B256, MptNode>), String> {
    // Step 1: Decode all RLP-encoded trie nodes and index by hash
    // IMPORTANT: Witness state contains both *state trie* nodes and *storage tries* nodes!
    let mut node_map: HashMap<MptNodeReference, MptNode> = HashMap::default();
    let mut node_by_hash: HashMap<B256, MptNode> = HashMap::default();
    let mut root_node: Option<MptNode> = None;

    for encoded in &witness.state {
        let node = MptNode::decode(encoded).expect("Valid MPT node in witness");
        let hash = keccak256(encoded);
        if hash == pre_state_root {
            root_node = Some(node.clone());
        }
        node_by_hash.insert(hash, node.clone());
        node_map.insert(node.reference(), node);
    }

    // Step 2: Use root_node or fallback to Digest
    let root = root_node.unwrap_or_else(|| MptNodeData::Digest(pre_state_root).into());

    // Build state trie.
    let mut raw_storage_tries = vec![];
    let state_trie = resolve_nodes(&root, &node_map);

    state_trie.for_each_leaves(|key, mut value| {
        let account = TrieAccount::decode(&mut value).unwrap();
        let hashed_address = B256::from_slice(key);
        raw_storage_tries.push((hashed_address, account.storage_root));
    });

    // Step 3: Build storage tries per account efficiently
    let mut storage_tries: HashMap<B256, MptNode> = HashMap::default();

    for (hashed_address, storage_root) in raw_storage_tries {
        let root_node = match node_by_hash.get(&storage_root).cloned() {
            Some(node) => node,
            None => {
                // An execution witness can include an account leaf (with non-empty storageRoot),
                // but omit its entire storage trie when that account's storage was
                // NOT touched during the block.
                continue;
            }
        };
        let storage_trie = resolve_nodes(&root_node, &node_map);

        if storage_trie.is_digest() {
            panic!("Could not resolve storage trie for {storage_root}");
        }

        // Insert resolved storage trie.
        storage_tries.insert(hashed_address, storage_trie);
    }

    // Step 3a: Verify that state_trie was built correctly - confirm tree hash with pre state root.
    validate_state_trie(&state_trie, pre_state_root);

    // Step 3b: Verify that each storage trie matches the declared storage_root in the state trie.
    validate_storage_tries(&state_trie, &storage_tries)?;

    Ok((state_trie, storage_tries))
}

// Validate that state_trie was built correctly - confirm tree hash with pre state root.
fn validate_state_trie(state_trie: &MptNode, pre_state_root: B256) {
    if state_trie.hash() != pre_state_root {
        panic!("Computed state root does not match pre_state_root");
    }
}

// Validates that each storage trie matches the declared storage_root in the state trie.
fn validate_storage_tries(
    state_trie: &MptNode,
    storage_tries: &HashMap<B256, MptNode>,
) -> Result<(), String> {
    for (hashed_address, storage_trie) in storage_tries.iter() {
        let account = state_trie
            .get_rlp::<TrieAccount>(hashed_address.as_slice())
            .map_err(|_| "Failed to decode account from state trie")?
            .ok_or("Account not found in state trie")?;

        let storage_root = account.storage_root;
        let actual_hash = storage_trie.hash();

        if storage_root != actual_hash {
            return Err(format!(
                "Mismatched storage root for address hash {:?}: expected {:?}, got {:?}",
                hashed_address, storage_root, actual_hash
            ));
        }
    }

    Ok(())
}
