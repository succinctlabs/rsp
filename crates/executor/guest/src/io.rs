use std::collections::HashMap;

use eyre::Result;
use reth_primitives::{revm_primitives::AccountInfo, Address, Block, B256, U256};
use reth_trie::AccountProof;
use rsp_primitives::account_proof::AccountProofWithBytecode;
use rsp_witness_db::WitnessDb;
use serde::{Deserialize, Serialize};

/// The input for the guest to execute a block and fully verify the STF (state transition function).
///
/// Instead of passing in the entire state, we only pass in the state roots along with merkle proofs
/// for the storage slots that were modified and accessed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuestExecutorInput {
    /// The current block (which will be executed inside the guest).
    pub current_block: Block,
    /// The previous block.
    pub previous_block: Block,
    /// The dirty storage proofs for the storage slots that were modified.
    pub dirty_storage_proofs: Vec<AccountProof>,
    /// The storage proofs for the storage slots that were accessed.
    pub used_storage_proofs: HashMap<Address, AccountProofWithBytecode>,
    /// The block hashes.
    pub block_hashes: HashMap<u64, B256>,
}

impl GuestExecutorInput {
    /// Creates a [WitnessDb] from a [GuestExecutorInput]. To do so, it verifies the used storage
    /// proofs and constructs the account and storage values.
    ///
    /// Note: This mutates the input and takes ownership of used storage proofs and block hashes
    /// to avoid unnecessary cloning.
    pub fn witness_db(&mut self) -> Result<WitnessDb> {
        let state_root: B256 = self.previous_block.state_root;

        let mut accounts = HashMap::new();
        let mut storage = HashMap::new();
        let used_storage_proofs = std::mem::take(&mut self.used_storage_proofs);
        for (address, proof) in used_storage_proofs {
            // Verify the storage proof.
            proof.verify(state_root)?;

            // Update the accounts.
            let account_info = proof.proof.info.unwrap();
            let account_info = AccountInfo {
                nonce: account_info.nonce,
                balance: account_info.balance,
                code_hash: account_info.bytecode_hash.unwrap(),
                code: Some(proof.code),
            };
            accounts.insert(address, account_info);

            // Update the storage.
            let storage_values: HashMap<U256, U256> = proof
                .proof
                .storage_proofs
                .into_iter()
                .map(|storage_proof| (storage_proof.key.into(), storage_proof.value))
                .collect();
            storage.insert(address, storage_values);
        }

        Ok(WitnessDb {
            accounts,
            storage,
            block_hashes: std::mem::take(&mut self.block_hashes),
            state_root: self.current_block.state_root,
        })
    }
}
