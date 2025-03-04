use std::sync::Arc;

use alloy_consensus::{BlockHeader, Header, TxReceipt};
use alloy_evm::EthEvmFactory;
use alloy_primitives::Bloom;
use reth_chainspec::ChainSpec;
use reth_evm::execute::{BasicBlockExecutor, BlockExecutionStrategyFactory, Executor};

use reth_evm_ethereum::EthEvmConfig;
use reth_execution_types::ExecutionOutcome;
use reth_primitives_traits::Block;
use reth_trie::KeccakKeyHasher;

use revm::database::WrapDatabaseRef;
use revm_primitives::Address;

use crate::{
    custom::CustomEvmFactory, error::ClientError, into_primitives::FromInput,
    io::ClientExecutorInput,
};

pub type EthClientExecutor = ClientExecutor<EthEvmConfig<CustomEvmFactory<EthEvmFactory>>>;

#[cfg(feature = "optimism")]
pub type OpClientExecutor = ClientExecutor<reth_optimism_evm::OpEvmConfig>;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone)]
pub struct ClientExecutor<F: BlockExecutionStrategyFactory> {
    block_execution_strategy_factory: F,
}

impl<F> ClientExecutor<F>
where
    F: BlockExecutionStrategyFactory,
    F::Primitives: FromInput,
{
    pub fn execute(
        &self,
        mut input: ClientExecutorInput<F::Primitives>,
    ) -> Result<Header, ClientError> {
        // Initialize the witnessed database with verified storage proofs.
        let db = profile!("initialize witness db", {
            let trie_db = input.witness_db().unwrap();
            WrapDatabaseRef(trie_db)
        });

        let block_executor =
            BasicBlockExecutor::new(self.block_execution_strategy_factory.clone(), db);

        let block = profile!("recover senders", {
            F::Primitives::from_input_block(input.current_block.clone())
                .try_into_recovered()
                .map_err(|_| ClientError::SignatureRecoveryFailed)
        })?;

        let execution_output = profile!("block execution", { block_executor.execute(&block) })?;

        // Accumulate the logs bloom.
        let mut logs_bloom = Bloom::default();
        profile!("accrue logs bloom", {
            execution_output.result.receipts.iter().for_each(|r| {
                logs_bloom.accrue_bloom(&r.bloom());
            })
        });

        // Convert the output to an execution outcome.
        let executor_outcome = ExecutionOutcome::new(
            execution_output.state,
            vec![execution_output.result.receipts],
            input.current_block.header().number(),
            vec![execution_output.result.requests],
        );

        // Verify the state root.
        let state_root = profile!("compute state root", {
            input.parent_state.update(&executor_outcome.hash_state_slow::<KeccakKeyHasher>());
            input.parent_state.state_root()
        });

        if state_root != input.current_block.header().state_root() {
            return Err(ClientError::MismatchedStateRoot);
        }

        // Derive the block header.
        // Note: the receipts root and gas used are verified by `validate_block_post_execution`.
        let header = Header {
            parent_hash: input.current_block.header().parent_hash(),
            ommers_hash: input.current_block.header().ommers_hash(),
            beneficiary: input.current_block.header().beneficiary(),
            state_root,
            transactions_root: input.current_block.header().transactions_root(),
            receipts_root: input.current_block.header().receipts_root(),
            logs_bloom,
            difficulty: input.current_block.header().difficulty(),
            number: input.current_block.header().number(),
            gas_limit: input.current_block.header().gas_limit(),
            gas_used: input.current_block.header().gas_used(),
            timestamp: input.current_block.header().timestamp(),
            extra_data: input.current_block.header().extra_data().clone(),
            mix_hash: input.current_block.header().mix_hash().unwrap(),
            nonce: input.current_block.header().nonce().unwrap(),
            base_fee_per_gas: input.current_block.header().base_fee_per_gas(),
            withdrawals_root: input.current_block.header().withdrawals_root(),
            blob_gas_used: input.current_block.header().blob_gas_used(),
            excess_blob_gas: input.current_block.header().excess_blob_gas(),
            parent_beacon_block_root: input.current_block.header().parent_beacon_block_root(),
            requests_hash: None,
        };

        Ok(header)
    }
}

impl EthClientExecutor {
    pub fn eth(chain_spec: Arc<ChainSpec>, custom_beneficiary: Option<Address>) -> Self {
        Self {
            block_execution_strategy_factory: EthEvmConfig::new_with_evm_factory(
                chain_spec,
                CustomEvmFactory::<EthEvmFactory>::new(custom_beneficiary),
            ),
        }
    }
}

#[cfg(feature = "optimism")]
impl OpClientExecutor {
    pub fn optimism(chain_spec: Arc<reth_optimism_chainspec::OpChainSpec>) -> Self {
        Self {
            block_execution_strategy_factory: reth_optimism_evm::OpEvmConfig::optimism(chain_spec),
        }
    }
}
