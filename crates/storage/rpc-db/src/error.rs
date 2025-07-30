use alloy_transport::TransportError;
use revm_primitives::{Address, U256};
use rsp_mpt::FromProofError;

/// Errors that can occur when interacting with the [RpcDb].
#[derive(Debug, thiserror::Error)]
pub enum RpcDbError {
    #[error("Transport Error: {}", .0)]
    Transport(#[from] TransportError),
    #[error("From proof Error: {}", .0)]
    FromProof(#[from] FromProofError),
    #[error("failed fetch proof at {0}: {1}")]
    GetProofError(Address, String),
    #[error("failed to fetch code at {0}: {1}")]
    GetCodeError(Address, String),
    #[error("failed to fetch storage at {0}, index {1}: {2}")]
    GetStorageError(Address, U256, String),
    #[error("failed to fetch block {0}: {1}")]
    GetBlockError(u64, String),
    #[error("failed to find block {0}")]
    BlockNotFound(u64),
    #[error("failed to find trie node preimage")]
    PreimageNotFound,
    #[error("poisoned lock")]
    Poisoned,
}
