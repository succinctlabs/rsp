use std::sync::Arc;

use alloy_consensus::{BlockHeader, Header};
use itertools::Itertools;
use reth_chainspec::ChainSpec;
use reth_consensus_common::validation::validate_body_against_header;
use reth_errors::BlockExecutionError;
use reth_evm::{
    execute::{BasicBlockExecutor, Executor},
    ConfigureEvm, OnStateHook,
};
use reth_evm_ethereum::EthEvmConfig;
use reth_execution_types::ExecutionOutcome;
use reth_primitives_traits::{Block, SealedHeader};
use reth_trie::KeccakKeyHasher;
use revm::database::WrapDatabaseRef;
use revm_primitives::Address;

use crate::{
    custom::CustomEvmFactory,
    error::ClientError,
    into_primitives::FromInput,
    io::{ClientExecutorInput, TrieDB, WitnessInput},
    tracking::OpCodesTrackingBlockExecutor,
    BlockValidator,
};

pub const DESERIALZE_INPUTS: &str = "deserialize inputs";
pub const INIT_WITNESS_DB: &str = "initialize witness db";
pub const RECOVER_SENDERS: &str = "recover senders";
pub const BLOCK_EXECUTION: &str = "block execution";
pub const VALIDATE_HEADER: &str = "validate header";
pub const VALIDATE_EXECUTION: &str = "validate block post-execution";
pub const COMPUTE_STATE_ROOT: &str = "compute state root";

pub type EthClientExecutor = ClientExecutor<EthEvmConfig<ChainSpec, CustomEvmFactory>, ChainSpec>;

#[cfg(feature = "optimism")]
pub type OpClientExecutor =
    ClientExecutor<reth_optimism_evm::OpEvmConfig, reth_optimism_chainspec::OpChainSpec>;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone)]
pub struct ClientExecutor<C: ConfigureEvm, CS> {
    evm_config: C,
    chain_spec: Arc<CS>,
}

impl<C, CS> ClientExecutor<C, CS>
where
    C: ConfigureEvm,
    C::Primitives: FromInput + BlockValidator<CS>,
{
    pub fn execute(
        &self,
        mut input: ClientExecutorInput<C::Primitives>,
    ) -> Result<Header, ClientError> {
        let sealed_headers = input.sealed_headers().collect::<Vec<_>>();

        // Initialize the witnessed database with verified storage proofs.
        let db = profile_report!(INIT_WITNESS_DB, {
            let trie_db = input.witness_db(&sealed_headers).unwrap();
            WrapDatabaseRef(trie_db)
        });

        let block_executor = BlockExecutor::new(self.evm_config.clone(), db, input.opcode_tracking);

        let block = profile_report!(RECOVER_SENDERS, {
            C::Primitives::from_input_block(input.current_block.clone())
                .try_into_recovered()
                .map_err(|_| ClientError::SignatureRecoveryFailed)
        })?;

        // Validate the blocks.
        profile_report!(VALIDATE_HEADER, {
            C::Primitives::validate_header(
                &SealedHeader::seal_slow(input.current_block.header().clone()),
                self.chain_spec.clone(),
            )
            .expect("The header is invalid");

            validate_body_against_header(block.body(), block.header())
                .expect("The block body is invalid against its header");

            for (header, parent) in sealed_headers.iter().tuple_windows() {
                C::Primitives::validate_header(parent, self.chain_spec.clone())
                    .expect("A parent header is invalid");

                C::Primitives::validate_header_against_parent(
                    header,
                    parent,
                    self.chain_spec.clone(),
                )
                .expect("The header is invalid against its parent");
            }
        });

        let execution_output =
            profile_report!(BLOCK_EXECUTION, { block_executor.execute(&block) })?;

        // Validate the block post execution.
        profile_report!(VALIDATE_EXECUTION, {
            C::Primitives::validate_block_post_execution(
                &block,
                self.chain_spec.clone(),
                &execution_output,
            )
        })?;

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
            logs_bloom: input.current_block.logs_bloom,
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
            requests_hash: input.current_block.header().requests_hash(),
        };

        Ok(header)
    }
}

impl EthClientExecutor {
    pub fn eth(chain_spec: Arc<ChainSpec>, custom_beneficiary: Option<Address>) -> Self {
        Self {
            evm_config: EthEvmConfig::new_with_evm_factory(
                chain_spec.clone(),
                CustomEvmFactory::new(custom_beneficiary),
            ),
            chain_spec,
        }
    }
}

#[cfg(feature = "optimism")]
impl OpClientExecutor {
    pub fn optimism(chain_spec: Arc<reth_optimism_chainspec::OpChainSpec>) -> Self {
        Self {
            evm_config: reth_optimism_evm::OpEvmConfig::optimism(chain_spec.clone()),
            chain_spec,
        }
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
