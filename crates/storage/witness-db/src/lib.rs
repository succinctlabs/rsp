use reth_primitives::{
    revm_primitives::{db::DatabaseRef, AccountInfo, Bytecode},
    B256,
};
use reth_storage_errors::provider::ProviderError;
use revm_primitives::{Address, HashMap, U256};
use rsp_primitives::DebugGet;
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

impl DebugGet for WitnessDb {
    fn debug_get(&self, key: &[u8]) -> Option<&[u8]> {
        self.storage.get(&key.into()).map(|v| v.as_slice())
    }
}
