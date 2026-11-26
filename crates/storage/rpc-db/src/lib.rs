#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_consensus::Header;
use alloy_provider::Network;
use async_trait::async_trait;
use evm2::{block::bundle_state::BundleState, evm::DatabaseRef};
use revm_state::Bytecode;
use rsp_mpt::EthereumState;

mod basic;
pub use basic::BasicRpcDb;

#[cfg(feature = "execution-witness")]
mod execution_witness;
#[cfg(feature = "execution-witness")]
pub use execution_witness::ExecutionWitnessRpcDb;

mod error;
pub use error::RpcDbError;

#[async_trait]
pub trait RpcDb<N: Network>: DatabaseRef {
    async fn state(&self, bundle_state: &BundleState) -> Result<EthereumState, RpcDbError>;

    /// Gets all account bytecodes in the existing guest input format.
    fn bytecodes(&self) -> Vec<Bytecode>;

    // Fetches the parent headers needed to constrain the BLOCKHASH opcode.
    async fn ancestor_headers(&self) -> Result<Vec<Header>, RpcDbError>;
}

/// Preserves the bytecode encoding used by cached client input.
fn cached_bytecode(code: &evm2::bytecode::Bytecode) -> Bytecode {
    match code.eip7702_address() {
        Some(address) => Bytecode::new_eip7702(address),
        None => Bytecode::new_legacy(code.original_bytes()),
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes};
    use evm2::bytecode::Bytecode;

    #[test]
    fn cached_bytecode_preserves_stored_bytes_and_delegation() {
        let delegated = Bytecode::new_eip7702(Address::repeat_byte(0x42)).original_bytes();
        for (stored, is_delegation) in [
            (Bytes::new(), false),
            (Bytes::from_static(&[0x60, 0x00, 0x5b]), false),
            // Predating EIP-3541, code can have the prefix without being a valid designator.
            (Bytes::from_static(&[0xef, 0x01]), false),
            (delegated, true),
        ] {
            let cached = super::cached_bytecode(&Bytecode::from_stored(stored.clone()));
            assert_eq!(cached.original_bytes(), stored);
            assert_eq!(cached.is_eip7702(), is_delegation);
        }
    }
}
