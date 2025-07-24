#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_provider::Network;
use async_trait::async_trait;
use revm_database::{BundleState, DatabaseRef};
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

    /// Gets all account bytecodes.
    fn bytecodes(&self) -> Vec<Bytecode>;

    // Fetches the parent headers needed to constrain the BLOCKHASH opcode.
    async fn ancestor_headers(&self) -> Result<Vec<N::HeaderResponse>, RpcDbError>;
}
