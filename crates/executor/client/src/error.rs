use alloy_primitives::{Address, FixedBytes};
use reth_consensus::ConsensusError;
use reth_evm::execute::BlockExecutionError;
use rsp_mpt::Error as MptError;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Failed to recover senders from signatures")]
    SignatureRecoveryFailed,
    #[error("Mismatched state root after executing the block")]
    MismatchedStateRoot,
    #[error("Mismatched storage root after executing the block")]
    MismatchedStorageRoot,
    #[error("unknown chain ID: {}", .0)]
    UnknownChainId(u64),
    #[error("Missing bytecode for account {}", .0)]
    MissingBytecode(Address),
    #[error("Missing trie for address {}", .0)]
    MissingTrie(Address),
    #[error("Invalid block number found in headers \n expected: {} found: {}", .0, .1)]
    InvalidHeaderBlockNumber(u64, u64),
    #[error("Invalid parent header found for block \n expected: {}, found: {}", .0, .1)]
    InvalidHeaderParentHash(FixedBytes<32>, FixedBytes<32>),
    #[error("Failed to validate post exectution state {}", 0)]
    PostExecutionError(#[from] ConsensusError),
    #[error("Block Execution Failed: {}", .0)]
    BlockExecutionError(#[from] BlockExecutionError),
    #[error("Mpt Error: {}", .0)]
    MptError(#[from] MptError),
    #[error("Failed to read the genesis file: {}", .0)]
    FailedToReadGenesisFile(#[from] std::io::Error),
    #[error("Failed to deserialize the genesis file: {}", .0)]
    FailedToDeserializeGenesisFile(#[from] serde_json::Error),
}
