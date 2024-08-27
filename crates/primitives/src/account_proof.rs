use alloy_rpc_types::EIP1186AccountProofResponse;
use reth_primitives::{revm_primitives::Bytecode, Account, B256};
use reth_trie::{AccountProof, StorageProof, EMPTY_ROOT_HASH};
use revm_primitives::keccak256;
use serde::{Deserialize, Serialize};

/// The account proof with the bytecode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountProofWithBytecode {
    /// The account proof.
    pub proof: AccountProof,
    /// The bytecode of the account.
    pub code: Bytecode,
}

impl AccountProofWithBytecode {
    pub fn from_eip1186_proof(proof: EIP1186AccountProofResponse, bytecode: Bytecode) -> Self {
        Self { proof: eip1186_proof_to_account_proof(proof), code: bytecode }
    }

    /// Verifies the account proof against the provided state root.
    pub fn verify(&self, state_root: B256) -> eyre::Result<()> {
        self.proof
            .verify(state_root)
            .map_err(|err| eyre::eyre!("Account proof verification failed: {err}"))?;
        if let Some(info) = &self.proof.info {
            if info.bytecode_hash.unwrap() != keccak256(self.code.bytes()) {
                return Err(eyre::eyre!("Code hash does not match the code"));
            }
        }
        Ok(())
    }
}

/// Converts an [EIP1186AccountProofResponse] to an [AccountProof].
pub fn eip1186_proof_to_account_proof(proof: EIP1186AccountProofResponse) -> AccountProof {
    let address = proof.address;
    let balance = proof.balance;
    let code_hash = proof.code_hash;
    let storage_root = proof.storage_hash;
    let account_proof = proof.account_proof;
    let storage_proofs = proof
        .storage_proof
        .into_iter()
        .map(|storage_proof| {
            let key = storage_proof.key;
            let value = storage_proof.value;
            let proof = storage_proof.proof;
            let mut sp = StorageProof::new(key.0);
            sp.value = value;
            sp.proof = proof;
            sp
        })
        .collect();

    let (storage_root, info) =
        if proof.nonce == 0 && balance.is_zero() && storage_root.is_zero() && code_hash.is_zero() {
            // Account does not exist in state. Return `None` here to prevent proof verification.
            (EMPTY_ROOT_HASH, None)
        } else {
            (
                storage_root,
                Some(Account { nonce: proof.nonce, balance, bytecode_hash: code_hash.into() }),
            )
        };

    AccountProof { address, info, proof: account_proof, storage_root, storage_proofs }
}
