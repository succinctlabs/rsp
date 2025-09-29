use alloy_primitives::{map::HashMap, B256, U256};
use reth_primitives_traits::Account;
use reth_trie::{HashedPostState, HashedStorage};
use rsp_mpt::EthereumState;

fn main() {
    // Create state with insertions that force ALL node types to appear
    let mut state =
        EthereumState { state_trie: Default::default(), storage_tries: HashMap::default() };

    let mut post_state = HashedPostState::default();

    // These specific addresses will create Branch, Extension, and Leaf nodes
    post_state.accounts.insert(
        B256::from([0xAA; 32]),
        Some(Account { nonce: 1, balance: U256::from(1000), bytecode_hash: Some(B256::ZERO) }),
    );
    post_state.accounts.insert(
        B256::from([0xBB; 32]),
        Some(Account { nonce: 2, balance: U256::from(2000), bytecode_hash: Some(B256::ZERO) }),
    );
    post_state.accounts.insert(
        B256::from([0xCC; 32]),
        Some(Account { nonce: 3, balance: U256::from(3000), bytecode_hash: Some(B256::ZERO) }),
    );

    // Add storage to create storage tries with multiple slots
    let mut storage = HashedStorage::new(false);
    storage.storage.insert(B256::from([0x01; 32]), U256::from(100));
    storage.storage.insert(B256::from([0x02; 32]), U256::from(200));
    post_state.storages.insert(B256::from([0xAA; 32]), storage);

    state.update(&post_state);

    // Force reference computation by calling hash on the tries
    // This populates the cached_reference field
    let _state_root = state.state_root();
    for storage in state.storage_tries.values() {
        let _ = storage.hash();
    }

    // Print the full state - this will show Branch, Extension, Leaf nodes with cached references
    println!("{:#?}", state);
}
