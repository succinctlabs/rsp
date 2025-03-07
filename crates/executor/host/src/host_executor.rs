use std::{collections::BTreeSet, sync::Arc};

use alloy_consensus::{BlockHeader, Header, TxReceipt};
use alloy_evm::EthEvmFactory;
use alloy_primitives::{Bloom, Sealable};
use alloy_provider::{Network, Provider};
use alloy_rpc_types::BlockTransactionsKind;
use reth_chainspec::ChainSpec;
use reth_evm::execute::{BasicBlockExecutor, BlockExecutionStrategyFactory, Executor};
use reth_evm_ethereum::EthEvmConfig;
use reth_execution_types::ExecutionOutcome;
use reth_optimism_evm::OpEvmConfig;
use reth_primitives_traits::{Block, BlockBody};
use reth_trie::KeccakKeyHasher;
use revm::database::CacheDB;
use revm_primitives::{Address, B256};
use rsp_client_executor::{
    custom::CustomEvmFactory, io::ClientExecutorInput, IntoInput, IntoPrimitives,
    ValidateBlockPostExecution,
};
use rsp_mpt::EthereumState;
use rsp_primitives::{account_proof::eip1186_proof_to_account_proof, genesis::Genesis};
use rsp_rpc_db::RpcDb;

use crate::HostError;

pub type EthHostExecutor = HostExecutor<EthEvmConfig<CustomEvmFactory<EthEvmFactory>>>;

pub type OpHostExecutor = HostExecutor<OpEvmConfig>;

/// An executor that fetches data from a [Provider] to execute blocks in the [ClientExecutor].
#[derive(Debug, Clone)]
pub struct HostExecutor<F: BlockExecutionStrategyFactory> {
    block_execution_strategy_factory: F,
}

impl EthHostExecutor {
    pub fn eth(chain_spec: Arc<ChainSpec>, custom_beneficiary: Option<Address>) -> Self {
        Self {
            block_execution_strategy_factory: EthEvmConfig::new_with_evm_factory(
                chain_spec,
                CustomEvmFactory::<EthEvmFactory>::new(custom_beneficiary),
            ),
        }
    }
}

impl OpHostExecutor {
    pub fn optimism(chain_spec: Arc<reth_optimism_chainspec::OpChainSpec>) -> Self {
        Self { block_execution_strategy_factory: OpEvmConfig::optimism(chain_spec) }
    }
}

impl<F: BlockExecutionStrategyFactory> HostExecutor<F> {
    /// Creates a new [HostExecutor].
    pub fn new(block_execution_strategy_factory: F) -> Self {
        Self { block_execution_strategy_factory }
    }

    /// Executes the block with the given block number.
    pub async fn execute<P, N>(
        &self,
        block_number: u64,
        rpc_db: &RpcDb<P, N>,
        provider: &P,
        genesis: Genesis,
        custom_beneficiary: Option<Address>,
        opcode_tracking: bool,
    ) -> Result<ClientExecutorInput<F::Primitives>, HostError>
    where
        F::Primitives: IntoPrimitives<N> + IntoInput + ValidateBlockPostExecution,
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

        let block_executor =
            BasicBlockExecutor::new(self.block_execution_strategy_factory.clone(), cache_db);

        // Execute the block and fetch all the necessary data along the way.
        tracing::info!(
            "executing the block with rpc db: block_number={}, transaction_count={}",
            block_number,
            current_block.body().transactions().len()
        );

        let block = current_block
            .clone()
            .try_into_recovered()
            .map_err(|_| HostError::FailedToRecoverSenders)
            .unwrap();

        let execution_output = block_executor.execute(&block)?;

        // Validate the block post execution.
        tracing::info!("validating the block post execution");
        F::Primitives::validate_block_post_execution(&block, &genesis, &execution_output)?;

        // Accumulate the logs bloom.
        tracing::info!("accumulating the logs bloom");
        let mut logs_bloom = Bloom::default();
        execution_output.result.receipts.iter().for_each(|r| {
            logs_bloom.accrue_bloom(&r.bloom());
        });

        // Convert the output to an execution outcome.
        let executor_outcome = ExecutionOutcome::new(
            execution_output.state,
            vec![execution_output.result.receipts],
            current_block.header().number(),
            vec![execution_output.result.requests],
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
        let oldest_ancestor = *rpc_db.oldest_ancestor.read().unwrap();
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
            genesis,
            custom_beneficiary,
            opcode_tracking,
        };
        tracing::info!("successfully generated client input");

        Ok(client_input)
    }
}
