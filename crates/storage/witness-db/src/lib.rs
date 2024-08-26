use reth_primitives::{
    revm_primitives::{db::DatabaseRef, AccountInfo, Bytecode},
    Bytes, B256,
};
use reth_storage_errors::provider::ProviderError;
use revm_primitives::{Address, HashMap, U256};
use rsp_primitives::storage::{ExtDatabaseRef, PreimageContext};
use serde::{Deserialize, Serialize};

/// A database used to witness state inside the zkVM.
#[derive(Debug, Serialize, Deserialize)]
pub struct WitnessDb {
    /// The state root.
    pub state_root: B256,
    /// The accounts.
    pub accounts: HashMap<Address, AccountInfo>,
    /// The storage values, indexed by account address and slot.
    pub storage: HashMap<Address, HashMap<U256, U256>>,
    /// The block hashes, indexed by block number.
    pub block_hashes: HashMap<u64, B256>,
    /// The trie node preimages, indexed by Keccak hash.
    pub trie_nodes: HashMap<B256, Bytes>,
}

impl DatabaseRef for WitnessDb {
    type Error = ProviderError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        Ok(self.accounts.get(&address).cloned())
    }

    fn code_by_hash_ref(&self, _code_hash: B256) -> Result<Bytecode, Self::Error> {
        unimplemented!()
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(*self.storage.get(&address).unwrap().get(&index).unwrap())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        Ok(*self.block_hashes.get(&number).unwrap())
    }
}

impl ExtDatabaseRef for WitnessDb {
    type Error = ProviderError;

    fn trie_node_ref(&self, hash: B256) -> Result<Bytes, Self::Error> {
        // TODO: avoid cloning
        Ok(self.trie_nodes.get(&hash).unwrap().to_owned())
    }

    fn trie_node_ref_with_context(
        &self,
        hash: B256,
        _context: PreimageContext,
    ) -> Result<Bytes, Self::Error> {
        self.trie_node_ref(hash)
    }
}
