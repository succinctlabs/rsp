use std::sync::Arc;

use alloy_consensus::{BlockHeader, Header, TxReceipt};
use alloy_primitives::Bloom;
use reth_chainspec::ChainSpec;
use reth_evm::execute::{BlockExecutionStrategy, BlockExecutionStrategyFactory};
use reth_evm_ethereum::execute::EthExecutionStrategyFactory;
use reth_execution_types::ExecutionOutcome;
use reth_primitives_traits::Block;
use reth_trie::KeccakKeyHasher;
use revm::db::WrapDatabaseRef;

use crate::{custom::CustomEthEvmConfig, error::ClientError, io::ClientExecutorInput, FromAny};

pub type EthClientExecutor = ClientExecutor<EthExecutionStrategyFactory<CustomEthEvmConfig>>;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone)]
pub struct ClientExecutor<F: BlockExecutionStrategyFactory> {
    block_execution_strategy_factory: F,
}

impl<F> ClientExecutor<F>
where
    F: BlockExecutionStrategyFactory,
    F::Primitives: FromAny,
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

        let mut strategy = self.block_execution_strategy_factory.create_strategy(db);

        let block = profile!("recover senders", {
            F::Primitives::from_input_block(input.current_block.clone())
                .try_into_recovered()
                .map_err(|_| ClientError::SignatureRecoveryFailed)
        })?;

        strategy.apply_pre_execution_changes(&block)?;

        let executor_output = profile!("execute", { strategy.execute_transactions(&block) })?;

        let requests = strategy.apply_post_execution_changes(&block, &executor_output.receipts)?;

        let state = strategy.finish();

        // Validate the block post execution.
        profile!("validate block post-execution", {
            strategy.validate_block_post_execution(&block, &executor_output.receipts, &requests)
        })?;

        drop(strategy);

        // Accumulate the logs bloom.
        let mut logs_bloom = Bloom::default();
        profile!("accrue logs bloom", {
            executor_output.receipts.iter().for_each(|r| {
                logs_bloom.accrue_bloom(&r.bloom());
            })
        });

        // Convert the output to an execution outcome.
        let executor_outcome = ExecutionOutcome::new(
            state,
            vec![executor_output.receipts],
            input.current_block.header.number(),
            vec![requests],
        );

        // Verify the state root.
        let state_root = profile!("compute state root", {
            input.parent_state.update(&executor_outcome.hash_state_slow::<KeccakKeyHasher>());
            input.parent_state.state_root()
        });

        if state_root != input.current_block.header.state_root() {
            return Err(ClientError::MismatchedStateRoot);
        }

        // Derive the block header.
        // Note: the receipts root and gas used are verified by `validate_block_post_execution`.
        let mut header = input.current_block.header.clone();
        header.parent_hash = input.parent_header().hash_slow();
        header.state_root = state_root;
        header.logs_bloom = logs_bloom;
        header.requests_hash = Some(executor_outcome.requests[0].requests_hash());

        Ok(input.current_block.header.clone())
    }
}

impl EthClientExecutor {
    pub fn eth(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            block_execution_strategy_factory: EthExecutionStrategyFactory::new(
                chain_spec.clone(),
                CustomEthEvmConfig::eth(chain_spec),
            ),
        }
    }
}
