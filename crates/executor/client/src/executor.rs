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

use crate::{
    custom::CustomEthEvmConfig, error::ClientError, into_primitives::FromInput,
    io::ClientExecutorInput,
};

pub type EthClientExecutor = ClientExecutor<EthExecutionStrategyFactory<CustomEthEvmConfig>>;

#[cfg(feature = "optimism")]
pub type OpClientExecutor = ClientExecutor<
    reth_optimism_evm::OpExecutionStrategyFactory<
        reth_optimism_primitives::OpPrimitives,
        reth_optimism_chainspec::OpChainSpec,
        crate::custom::CustomOpEvmConfig,
    >,
>;

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
            input.current_block.header().number(),
            vec![requests],
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
    pub fn eth(chain_spec: Arc<ChainSpec>) -> Self {
        Self {
            block_execution_strategy_factory: EthExecutionStrategyFactory::new(
                chain_spec.clone(),
                CustomEthEvmConfig::eth(chain_spec),
            ),
        }
    }
}

#[cfg(feature = "optimism")]
impl OpClientExecutor {
    pub fn optimism(chain_spec: Arc<reth_optimism_chainspec::OpChainSpec>) -> Self {
        Self {
            block_execution_strategy_factory: reth_optimism_evm::OpExecutionStrategyFactory::new(
                chain_spec.clone(),
                crate::custom::CustomOpEvmConfig::optimism(chain_spec),
                reth_optimism_evm::BasicOpReceiptBuilder::default(),
            ),
        }
    }
}
