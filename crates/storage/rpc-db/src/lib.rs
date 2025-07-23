#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_provider::Network;
use async_trait::async_trait;
use revm_database::{BundleState, DatabaseRef};
use revm_primitives::B256;
use revm_state::Bytecode;
use rsp_mpt::EthereumState;

mod basic;
pub use basic::BasicRpcDb;

mod error;
pub use error::RpcDbError;

#[async_trait]
pub trait RpcDb<N: Network>: DatabaseRef {
    async fn state(
        &self,
        bundle_state: &BundleState,
        block_number: u64,
        parent_state_root: B256,
    ) -> Result<EthereumState, RpcDbError>;

    /// Gets all account bytecodes.
    fn bytecodes(&self) -> Vec<Bytecode>;

    // Fetches the parent headers needed to constrain the BLOCKHASH opcode.
    async fn ancestor_headers(&self) -> Result<Vec<N::HeaderResponse>, RpcDbError>;
}
