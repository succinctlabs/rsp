use alloy_consensus::{Block, Header};
use alloy_network::{Ethereum, Network};
use reth_chainspec::ChainSpec;
use reth_errors::ConsensusError;
use reth_execution_types::BlockExecutionOutput;
use reth_primitives::{EthPrimitives, NodePrimitives, RecoveredBlock};
use rsp_primitives::genesis::Genesis;

pub trait IntoPrimitives<N: Network>: NodePrimitives {
    fn into_primitive_block(block: N::BlockResponse) -> Self::Block;

    fn into_primitive_header(block: N::BlockResponse) -> Header;
}

pub trait FromInput: NodePrimitives {
    fn from_input_block(block: Block<Self::SignedTx>) -> Self::Block;
}

pub trait IntoInput: NodePrimitives {
    fn into_input_block(block: Self::Block) -> Block<Self::SignedTx>;
}

pub trait ValidateBlockPostExecution: NodePrimitives {
    fn validate_block_post_execution(
        block: &RecoveredBlock<Self::Block>,
        genesis: &Genesis,
        execution_output: &BlockExecutionOutput<Self::Receipt>,
    ) -> Result<(), ConsensusError>;
}

impl IntoPrimitives<Ethereum> for EthPrimitives {
    fn into_primitive_block(block: alloy_rpc_types::Block) -> Self::Block {
        let block = block.map_transactions(|tx| tx.inner.into());
        block.into_consensus()
    }

    fn into_primitive_header(block: alloy_rpc_types::Block) -> Header {
        block.header.into()
    }
}

impl FromInput for EthPrimitives {
    fn from_input_block(block: Block<Self::SignedTx>) -> Self::Block {
        block
    }
}

impl IntoInput for EthPrimitives {
    fn into_input_block(block: Self::Block) -> Block<Self::SignedTx> {
        block
    }
}

impl ValidateBlockPostExecution for EthPrimitives {
    fn validate_block_post_execution(
        block: &RecoveredBlock<Self::Block>,
        genesis: &Genesis,
        execution_output: &BlockExecutionOutput<Self::Receipt>,
    ) -> Result<(), ConsensusError> {
        let chain_spec = ChainSpec::try_from(genesis).unwrap();
        reth_ethereum_consensus::validate_block_post_execution(
            block,
            &chain_spec,
            &execution_output.result.receipts,
            &execution_output.result.requests,
        )
    }
}

#[cfg(feature = "optimism")]
impl IntoPrimitives<op_alloy_network::Optimism> for reth_optimism_primitives::OpPrimitives {
    fn into_primitive_block(
        block: alloy_rpc_types::Block<op_alloy_rpc_types::Transaction>,
    ) -> Self::Block {
        let block = block.map_transactions(|tx| tx.inner.inner.into());
        block.into_consensus()
    }

    fn into_primitive_header(
        block: alloy_rpc_types::Block<op_alloy_rpc_types::Transaction>,
    ) -> Header {
        block.header.into()
    }
}

#[cfg(feature = "optimism")]
impl FromInput for reth_optimism_primitives::OpPrimitives {
    fn from_input_block(block: Block<Self::SignedTx>) -> Self::Block {
        block
    }
}

#[cfg(feature = "optimism")]
impl IntoInput for reth_optimism_primitives::OpPrimitives {
    fn into_input_block(block: Self::Block) -> Block<Self::SignedTx> {
        block
    }
}

#[cfg(feature = "optimism")]
impl ValidateBlockPostExecution for reth_optimism_primitives::OpPrimitives {
    fn validate_block_post_execution(
        block: &RecoveredBlock<Self::Block>,
        genesis: &Genesis,
        execution_output: &BlockExecutionOutput<Self::Receipt>,
    ) -> Result<(), ConsensusError> {
        let chain_spec = reth_optimism_chainspec::OpChainSpec::try_from(genesis).unwrap();
        reth_optimism_consensus::validate_block_post_execution(
            block.header(),
            &chain_spec,
            &execution_output.result.receipts,
        )
    }
}
