#![warn(unused_crate_dependencies)]

use account_proof::eip1186_proof_to_account_proof;
use alloy_consensus::Header;
use alloy_genesis::Genesis;
use alloy_primitives::Bloom;
use alloy_provider::Network;
use alloy_rpc_types::BlockTransactionsKind;
pub use error::Error as HostError;
use reth_chainspec::ChainSpec;
use reth_optimism_chainspec::OpChainSpec;
use reth_optimism_evm::BasicOpReceiptBuilder;
use reth_optimism_evm::OpExecutionStrategyFactory;
use reth_optimism_primitives::OpPrimitives;
use reth_trie::KeccakKeyHasher;

use alloy_consensus::BlockHeader;
use alloy_consensus::TxReceipt;
use alloy_primitives::Sealable;
use alloy_provider::Provider;
use reth_evm::execute::{BlockExecutionStrategy, BlockExecutionStrategyFactory};
use reth_evm_ethereum::execute::EthExecutionStrategyFactory;
use reth_execution_types::ExecutionOutcome;
use reth_primitives_traits::{Block as BlockTrait, BlockBody};
use revm::db::CacheDB;
use revm_primitives::Address;
use revm_primitives::B256;
use rsp_client_executor::custom::CustomEthEvmConfig;
use rsp_client_executor::custom::CustomOpEvmConfig;
use rsp_client_executor::io::ClientExecutorInput;
use rsp_client_executor::IntoInput;
use rsp_client_executor::IntoPrimitives;
use rsp_mpt::EthereumState;
use rsp_rpc_db::RpcDb;
use std::collections::BTreeSet;
use std::sync::Arc;

mod account_proof;

mod error;

pub type EthHostExecutor = HostExecutor<EthExecutionStrategyFactory<CustomEthEvmConfig>>;

pub type OpHostExecutor =
    HostExecutor<OpExecutionStrategyFactory<OpPrimitives, OpChainSpec, CustomOpEvmConfig>>;

pub fn create_eth_block_execution_strategy_factory(
    genesis: Genesis,
    custom_beneficiary: Option<Address>,
) -> EthExecutionStrategyFactory<CustomEthEvmConfig> {
    let chain_spec = Arc::new(ChainSpec::from_genesis(genesis));

    EthExecutionStrategyFactory::new(
        chain_spec.clone(),
        CustomEthEvmConfig::eth(chain_spec, custom_beneficiary),
    )
}

pub fn create_op_block_execution_strategy_factory(
    genesis: Genesis,
) -> OpExecutionStrategyFactory<OpPrimitives, OpChainSpec, CustomOpEvmConfig> {
    let chain_spec = Arc::new(OpChainSpec::from_genesis(genesis));

    OpExecutionStrategyFactory::new(
        chain_spec.clone(),
        CustomOpEvmConfig::optimism(chain_spec),
        BasicOpReceiptBuilder::default(),
    )
}

/// An executor that fetches data from a [Provider] to execute blocks in the [ClientExecutor].
#[derive(Debug, Clone)]
pub struct HostExecutor<F: BlockExecutionStrategyFactory> {
    block_execution_strategy_factory: F,
}

impl EthHostExecutor {
    pub fn eth(chain_spec: Arc<ChainSpec>, custom_beneficiary: Option<Address>) -> Self {
        Self {
            block_execution_strategy_factory: EthExecutionStrategyFactory::new(
                chain_spec.clone(),
                CustomEthEvmConfig::eth(chain_spec, custom_beneficiary),
            ),
        }
    }
}

impl OpHostExecutor {
    pub fn optimism(chain_spec: Arc<reth_optimism_chainspec::OpChainSpec>) -> Self {
        Self {
            block_execution_strategy_factory: OpExecutionStrategyFactory::new(
                chain_spec.clone(),
                CustomOpEvmConfig::optimism(chain_spec),
                BasicOpReceiptBuilder::default(),
            ),
        }
    }
}

impl<F: BlockExecutionStrategyFactory> HostExecutor<F> {
    /// Creates a new [HostExecutor].
    pub fn new(block_execution_strategy_factory: F) -> Self {
        Self { block_execution_strategy_factory }
    }

    /// Executes the block with the given block number.
    pub async fn execute<'a, P, N>(
        &self,
        block_number: u64,
        rpc_db: &'a RpcDb<P, N>,
        provider: &P,
        genesis_json: String,
        custom_beneficiary: Option<Address>,
    ) -> Result<ClientExecutorInput<F::Primitives>, HostError>
    where
        F::Primitives: IntoPrimitives<N> + IntoInput,
        P: Provider<N> + Clone,
        N: Network,
    {
        // Fetch the current block and the previous block from the provider.
        tracing::info!("fetching the current block and the previous block");
        let current_block = provider
            .get_block_by_number(block_number.into(), BlockTransactionsKind::Full)
            .await?
            .ok_or(HostError::ExpectedBlock(block_number))
            .map(F::Primitives::into_primitive_block)?;

        let previous_block = provider
            .get_block_by_number((block_number - 1).into(), BlockTransactionsKind::Full)
            .await?
            .ok_or(HostError::ExpectedBlock(block_number))
            .map(F::Primitives::into_primitive_block)?;

        // Setup the database for the block executor.
        tracing::info!("setting up the database for the block executor");
        let cache_db = CacheDB::new(rpc_db);

        // Execute the block and fetch all the necessary data along the way.
        tracing::info!(
            "executing the block and with rpc db: block_number={}, transaction_count={}",
            block_number,
            current_block.body().transactions().len()
        );

        let executor_block_input = current_block
            .clone()
            .try_into_recovered()
            .map_err(|_| HostError::FailedToRecoverSenders)
            .unwrap();

        let mut strategy = self.block_execution_strategy_factory.create_strategy(cache_db);

        strategy.apply_pre_execution_changes(&executor_block_input)?;

        let executor_output = strategy.execute_transactions(&executor_block_input)?;

        let requests = strategy
            .apply_post_execution_changes(&executor_block_input, &executor_output.receipts)?;

        let state = strategy.finish();

        // Validate the block post execution.
        tracing::info!("validating the block post execution");
        strategy.validate_block_post_execution(
            &executor_block_input,
            &executor_output.receipts,
            &requests,
        )?;

        // Accumulate the logs bloom.
        tracing::info!("accumulating the logs bloom");
        let mut logs_bloom = Bloom::default();
        executor_output.receipts.iter().for_each(|r| {
            logs_bloom.accrue_bloom(&r.bloom());
        });

        // Convert the output to an execution outcome.
        let executor_outcome = ExecutionOutcome::new(
            state,
            vec![executor_output.receipts],
            current_block.header().number(),
            vec![requests],
        );

        let state_requests = rpc_db.get_state_requests();

        // For every account we touched, fetch the storage proofs for all the slots we touched.
        tracing::info!("fetching storage proofs");
        let mut before_storage_proofs = Vec::new();
        let mut after_storage_proofs = Vec::new();

        for (address, used_keys) in state_requests.iter() {
            let modified_keys = executor_outcome
                .state()
                .state
                .get(address)
                .map(|account| {
                    account.storage.keys().map(|key| B256::from(*key)).collect::<BTreeSet<_>>()
                })
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();

            let keys = used_keys
                .iter()
                .map(|key| B256::from(*key))
                .chain(modified_keys.clone().into_iter())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();

            let storage_proof = provider
                .get_proof(*address, keys.clone())
                .block_id((block_number - 1).into())
                .await?;
            before_storage_proofs.push(eip1186_proof_to_account_proof(storage_proof));

            let storage_proof =
                provider.get_proof(*address, modified_keys).block_id((block_number).into()).await?;
            after_storage_proofs.push(eip1186_proof_to_account_proof(storage_proof));
        }

        let state = EthereumState::from_transition_proofs(
            previous_block.header().state_root(),
            &before_storage_proofs.iter().map(|item| (item.address, item.clone())).collect(),
            &after_storage_proofs.iter().map(|item| (item.address, item.clone())).collect(),
        )?;

        // Verify the state root.
        tracing::info!("verifying the state root");
        let state_root = {
            let mut mutated_state = state.clone();
            mutated_state.update(&executor_outcome.hash_state_slow::<KeccakKeyHasher>());
            mutated_state.state_root()
        };
        if state_root != current_block.header().state_root() {
            return Err(HostError::StateRootMismatch(
                state_root,
                current_block.header().state_root(),
            ));
        }

        // Derive the block header.
        //
        // Note: the receipts root and gas used are verified by `validate_block_post_execution`.
        let header = Header {
            parent_hash: current_block.header().parent_hash(),
            ommers_hash: current_block.header().ommers_hash(),
            beneficiary: current_block.header().beneficiary(),
            state_root,
            transactions_root: current_block.header().transactions_root(),
            receipts_root: current_block.header().receipts_root(),
            logs_bloom,
            difficulty: current_block.header().difficulty(),
            number: current_block.header().number(),
            gas_limit: current_block.header().gas_limit(),
            gas_used: current_block.header().gas_used(),
            timestamp: current_block.header().timestamp(),
            extra_data: current_block.header().extra_data().clone(),
            mix_hash: current_block.header().mix_hash().unwrap(),
            nonce: current_block.header().nonce().unwrap(),
            base_fee_per_gas: current_block.header().base_fee_per_gas(),
            withdrawals_root: current_block.header().withdrawals_root(),
            blob_gas_used: current_block.header().blob_gas_used(),
            excess_blob_gas: current_block.header().excess_blob_gas(),
            parent_beacon_block_root: current_block.header().parent_beacon_block_root(),
            requests_hash: None,
        };

        // Assert the derived header is correct.
        let constructed_header_hash = header.hash_slow();
        let target_hash = current_block.header().hash_slow();
        if constructed_header_hash != target_hash {
            return Err(HostError::HeaderMismatch(constructed_header_hash, target_hash));
        }

        // Log the result.
        tracing::info!(
            "successfully executed block: block_number={}, block_hash={}, state_root={}",
            current_block.header().number(),
            constructed_header_hash,
            state_root
        );

        // Fetch the parent headers needed to constrain the BLOCKHASH opcode.
        let oldest_ancestor = *rpc_db.oldest_ancestor.borrow();
        let mut ancestor_headers = vec![];
        tracing::info!("fetching {} ancestor headers", block_number - oldest_ancestor);
        for height in (oldest_ancestor..=(block_number - 1)).rev() {
            let block = provider
                .get_block_by_number(height.into(), BlockTransactionsKind::Hashes)
                .await?
                .ok_or(HostError::ExpectedBlock(height))?;

            ancestor_headers.push(F::Primitives::into_primitive_header(block))
        }

        // Create the client input.
        let client_input = ClientExecutorInput {
            current_block: F::Primitives::into_input_block(current_block),
            ancestor_headers,
            parent_state: state,
            state_requests,
            bytecodes: rpc_db.get_bytecodes(),
            genesis_json,
            custom_beneficiary,
        };
        tracing::info!("successfully generated client input");

        Ok(client_input)
    }
}
