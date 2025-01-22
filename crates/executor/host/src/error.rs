use alloy_rpc_types::ConversionError;
use alloy_transport::TransportError;
use reth_errors::BlockExecutionError;
use revm_primitives::B256;
use rsp_mpt::FromProofError;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to parse blocks into executor friendly format {}", .0)]
    ParseError(#[from] ConversionError),
    #[error("Transport Error: {}", .0)]
    Transport(#[from] TransportError),
    #[error("Failed to recover senders from RPC block data")]
    FailedToRecoverSenders,
    #[error("Failed to validate post execution state")]
    PostExecutionCheck(#[from] reth_errors::ConsensusError),
    #[error("Local Execution Failed {}", .0)]
    ExecutionFailed(#[from] BlockExecutionError),
    #[error("Failed to construct a valid state trie from RPC data {}", .0)]
    FromProof(#[from] FromProofError),
    #[error("RPC didnt have expected block height {}", .0)]
    ExpectedBlock(u64),
    #[error("Header Mismatch \n found {} expected {}", .0, .1)]
    HeaderMismatch(B256, B256),
    #[error("State root mismatch after local execution \n found {} expected {}", .0, .1)]
    StateRootMismatch(B256, B256),
    #[error("Failed to read the genesis file: {}", .0)]
    FailedToReadGenesisFile(#[from] std::io::Error),
}
