#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_primitives::B256;
use reth_storage_errors::provider::ProviderError;
use revm_database_interface::{DatabaseRef, ErasedError};
use revm_primitives::{Address, HashMap, U256};
use revm_state::{AccountInfo, Bytecode};
use serde::{Deserialize, Serialize};

/// A database used to witness state inside the zkVM.
#[derive(Debug, Serialize, Deserialize)]
pub struct WitnessDb {
    /// The accounts.
    pub accounts: HashMap<Address, AccountInfo>,
    /// The storage values, indexed by account address and slot.
    pub storage: HashMap<Address, HashMap<U256, U256>>,
    /// The block hashes, indexed by block number.
    pub block_hashes: HashMap<u64, B256>,
}

impl DatabaseRef for WitnessDb {
    type Error = ErasedError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.accounts.get(&address).cloned().map(Some).ok_or_else(|| {
            ErasedError::new(ProviderError::TrieWitnessError(format!("Missing account {address}")))
        })
    }

    fn code_by_hash_ref(&self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Err(ErasedError::new(ProviderError::TrieWitnessError(format!(
            "WitnessDb does not support code lookup by hash: {code_hash}"
        ))))
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.storage.get(&address).and_then(|slots| slots.get(&index)).copied().ok_or_else(|| {
            ErasedError::new(ProviderError::TrieWitnessError(format!(
                "Missing storage slot {index} for {address}"
            )))
        })
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        self.block_hashes.get(&number).copied().ok_or_else(|| {
            ErasedError::new(ProviderError::TrieWitnessError(format!(
                "Missing block hash for {number}"
            )))
        })
    }
}
