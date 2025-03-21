use std::sync::Arc;

use alloy_consensus::{BlockHeader, Header, TxReceipt};
use alloy_evm::EthEvmFactory;
use alloy_primitives::Bloom;
use reth_chainspec::ChainSpec;
use reth_errors::BlockExecutionError;
use reth_evm::{
    execute::{BasicBlockExecutor, Executor},
    ConfigureEvm, OnStateHook,
};
use reth_evm_ethereum::EthEvmConfig;
use reth_execution_types::ExecutionOutcome;
use reth_primitives_traits::Block;
use reth_trie::KeccakKeyHasher;
use revm::database::WrapDatabaseRef;
use revm_primitives::Address;

use crate::{
    custom::CustomEvmFactory,
    error::ClientError,
    into_primitives::FromInput,
    io::{ClientExecutorInput, TrieDB},
    tracking::OpCodesTrackingBlockExecutor,
    ValidateBlockPostExecution,
};

pub const DESERIALZE_INPUTS: &str = "deserialize inputs";
pub const INIT_WITNESS_DB: &str = "initialize witness db";
pub const RECOVER_SENDERS: &str = "recover senders";
pub const BLOCK_EXECUTION: &str = "block execution";
pub const VALIDATE_EXECUTION: &str = "validate block post-execution";
pub const ACCRUE_LOG_BLOOM: &str = "accrue logs bloom";
pub const COMPUTE_STATE_ROOT: &str = "compute state root";

pub type EthClientExecutor = ClientExecutor<EthEvmConfig<CustomEvmFactory<EthEvmFactory>>>;

#[cfg(feature = "optimism")]
pub type OpClientExecutor = ClientExecutor<reth_optimism_evm::OpEvmConfig>;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone)]
pub struct ClientExecutor<C: ConfigureEvm> {
    evm_config: C,
}

impl<C> ClientExecutor<C>
where
    C: ConfigureEvm,
    C::Primitives: FromInput + ValidateBlockPostExecution,
{
    pub fn execute(
        &self,
        mut input: ClientExecutorInput<C::Primitives>,
    ) -> Result<Header, ClientError> {
        // Initialize the witnessed database with verified storage proofs.
        let db = profile_report!(INIT_WITNESS_DB, {
            let trie_db = input.witness_db().unwrap();
            WrapDatabaseRef(trie_db)
        });

        let block_executor = BlockExecutor::new(self.evm_config.clone(), db, input.opcode_tracking);

        let block = profile_report!(RECOVER_SENDERS, {
            C::Primitives::from_input_block(input.current_block.clone())
                .try_into_recovered()
                .map_err(|_| ClientError::SignatureRecoveryFailed)
        })?;

        let execution_output =
            profile_report!(BLOCK_EXECUTION, { block_executor.execute(&block) })?;

        // Validate the block post execution.
        profile_report!(VALIDATE_EXECUTION, {
            C::Primitives::validate_block_post_execution(&block, &input.genesis, &execution_output)
        })?;

        // Accumulate the logs bloom.
        let mut logs_bloom = Bloom::default();
        profile_report!(ACCRUE_LOG_BLOOM, {
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
        let state_root = profile_report!(COMPUTE_STATE_ROOT, {
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
            evm_config: EthEvmConfig::new_with_evm_factory(
                chain_spec,
                CustomEvmFactory::<EthEvmFactory>::new(custom_beneficiary),
            ),
        }
    }
}

#[cfg(feature = "optimism")]
impl OpClientExecutor {
    pub fn optimism(chain_spec: Arc<reth_optimism_chainspec::OpChainSpec>) -> Self {
        Self { evm_config: reth_optimism_evm::OpEvmConfig::optimism(chain_spec) }
    }
}

enum BlockExecutor<'a, C> {
    Basic(BasicBlockExecutor<C, WrapDatabaseRef<TrieDB<'a>>>),
    OpcodeTracking(OpCodesTrackingBlockExecutor<C, WrapDatabaseRef<TrieDB<'a>>>),
}

impl<'a, C: ConfigureEvm> BlockExecutor<'a, C> {
    fn new(strategy_factory: C, db: WrapDatabaseRef<TrieDB<'a>>, opcode_tracking: bool) -> Self {
        if opcode_tracking {
            Self::OpcodeTracking(OpCodesTrackingBlockExecutor::new(strategy_factory, db))
        } else {
            Self::Basic(BasicBlockExecutor::new(strategy_factory, db))
        }
    }
}

impl<'a, C> Executor<WrapDatabaseRef<TrieDB<'a>>> for BlockExecutor<'a, C>
where
    C: ConfigureEvm,
{
    type Primitives = C::Primitives;
    type Error = BlockExecutionError;

    fn execute_one(
        &mut self,
        block: &reth_primitives_traits::RecoveredBlock<
            <Self::Primitives as reth_primitives_traits::NodePrimitives>::Block,
        >,
    ) -> Result<
        reth_execution_types::BlockExecutionResult<
            <Self::Primitives as reth_primitives_traits::NodePrimitives>::Receipt,
        >,
        Self::Error,
    > {
        match self {
            BlockExecutor::Basic(basic_block_executor) => basic_block_executor.execute_one(block),
            BlockExecutor::OpcodeTracking(op_codes_tracking_block_executor) => {
                op_codes_tracking_block_executor.execute_one(block)
            }
        }
    }

    fn execute_one_with_state_hook<H>(
        &mut self,
        block: &reth_primitives_traits::RecoveredBlock<
            <Self::Primitives as reth_primitives_traits::NodePrimitives>::Block,
        >,
        state_hook: H,
    ) -> Result<
        reth_execution_types::BlockExecutionResult<
            <Self::Primitives as reth_primitives_traits::NodePrimitives>::Receipt,
        >,
        Self::Error,
    >
    where
        H: OnStateHook + 'static,
    {
        match self {
            BlockExecutor::Basic(basic_block_executor) => {
                basic_block_executor.execute_one_with_state_hook(block, state_hook)
            }
            BlockExecutor::OpcodeTracking(op_codes_tracking_block_executor) => {
                op_codes_tracking_block_executor.execute_one_with_state_hook(block, state_hook)
            }
        }
    }

    fn into_state(self) -> revm::database::State<WrapDatabaseRef<TrieDB<'a>>> {
        match self {
            BlockExecutor::Basic(basic_block_executor) => basic_block_executor.into_state(),
            BlockExecutor::OpcodeTracking(op_codes_tracking_block_executor) => {
                op_codes_tracking_block_executor.into_state()
            }
        }
    }

    fn size_hint(&self) -> usize {
        match self {
            BlockExecutor::Basic(basic_block_executor) => basic_block_executor.size_hint(),
            BlockExecutor::OpcodeTracking(op_codes_tracking_block_executor) => {
                op_codes_tracking_block_executor.size_hint()
            }
        }
    }
}
