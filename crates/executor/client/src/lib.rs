#![feature(stmt_expr_attributes)]

pub mod io;
#[macro_use]
pub mod utils;

use std::fmt::Display;

use eyre::eyre;
use io::ClientExecutorInput;
use reth_chainspec::ChainSpec;
use reth_errors::ProviderError;
use reth_ethereum_consensus::validate_block_post_execution as validate_block_post_execution_ethereum;
use reth_evm::execute::{BlockExecutionOutput, BlockExecutorProvider, Executor};
use reth_evm_ethereum::{execute::EthExecutorProvider, EthEvmConfig};
use reth_evm_optimism::{OpExecutorProvider, OptimismEvmConfig};
use reth_execution_types::ExecutionOutcome;
use reth_optimism_consensus::validate_block_post_execution as validate_block_post_execution_optimism;
use reth_primitives::{proofs, BlockWithSenders, Bloom, Header, Receipt, Receipts, Request};
use revm::{db::CacheDB, Database};
use revm_primitives::U256;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone, Default)]
pub struct ClientExecutor;

/// Trait for representing different execution/validation rules of different chain variants. This
/// allows for dead code elimination to minimize the ELF size for each variant.
pub trait Variant {
    fn spec() -> ChainSpec;

    fn execute<DB>(
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> eyre::Result<BlockExecutionOutput<Receipt>>
    where
        DB: Database<Error: Into<ProviderError> + Display>;

    fn validate_block_post_execution(
        block: &BlockWithSenders,
        chain_spec: &ChainSpec,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> eyre::Result<()>;
}

/// Implementation for Ethereum-specific execution/validation logic.
#[derive(Debug)]
pub struct EthereumVariant;

/// Implementation for Optimism-specific execution/validation logic.
#[derive(Debug)]
pub struct OptimismVariant;

impl ClientExecutor {
    pub fn execute<V>(&self, mut input: ClientExecutorInput) -> eyre::Result<Header>
    where
        V: Variant,
    {
        // Initialize the witnessed database with verified storage proofs.
        let witness_db = input.witness_db()?;
        let cache_db = CacheDB::new(&witness_db);

        // Execute the block.
        let spec = V::spec();
        let executor_block_input = profile!("recover senders", {
            input
                .current_block
                .clone()
                .with_recovered_senders()
                .ok_or(eyre!("failed to recover senders"))
        })?;
        let executor_difficulty = input.current_block.header.difficulty;
        let executor_output = profile!("execute", {
            V::execute(&executor_block_input, executor_difficulty, cache_db)
        })?;

        // Validate the block post execution.
        profile!("validate block post-execution", {
            V::validate_block_post_execution(
                &executor_block_input,
                &spec,
                &executor_output.receipts,
                &executor_output.requests,
            )
        })?;

        // Accumulate the logs bloom.
        let mut logs_bloom = Bloom::default();
        profile!("accrue logs bloom", {
            executor_output.receipts.iter().for_each(|r| {
                logs_bloom.accrue_bloom(&r.bloom_slow());
            })
        });

        // Convert the output to an execution outcome.
        let executor_outcome = ExecutionOutcome::new(
            executor_output.state,
            Receipts::from(executor_output.receipts),
            input.current_block.header.number,
            vec![executor_output.requests.into()],
        );

        // Verify the state root.
        let state_root = profile!("compute state root", {
            rsp_mpt::compute_state_root(&executor_outcome, &input.dirty_storage_proofs, &witness_db)
        })?;
        if state_root != input.current_block.state_root {
            eyre::bail!("mismatched state root");
        }

        // Derive the block header.
        //
        // Note: the receipts root and gas used are verified by `validate_block_post_execution`.
        let mut header = input.current_block.header.clone();
        header.parent_hash = input.previous_block.hash_slow();
        header.ommers_hash = proofs::calculate_ommers_root(&input.current_block.ommers);
        header.state_root = input.current_block.state_root;
        header.transactions_root = proofs::calculate_transaction_root(&input.current_block.body);
        header.receipts_root = input.current_block.header.receipts_root;
        header.withdrawals_root = input
            .current_block
            .withdrawals
            .clone()
            .map(|w| proofs::calculate_withdrawals_root(w.into_inner().as_slice()));
        header.logs_bloom = logs_bloom;
        header.requests_root =
            input.current_block.requests.as_ref().map(|r| proofs::calculate_requests_root(&r.0));

        Ok(header)
    }
}

impl Variant for EthereumVariant {
    fn spec() -> ChainSpec {
        rsp_primitives::chain_spec::mainnet().unwrap()
    }

    fn execute<DB>(
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> eyre::Result<BlockExecutionOutput<Receipt>>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        Ok(EthExecutorProvider::new(Self::spec().into(), EthEvmConfig::default())
            .executor(cache_db)
            .execute((executor_block_input, executor_difficulty).into())?)
    }

    fn validate_block_post_execution(
        block: &BlockWithSenders,
        chain_spec: &ChainSpec,
        receipts: &[Receipt],
        requests: &[Request],
    ) -> eyre::Result<()> {
        Ok(validate_block_post_execution_ethereum(block, chain_spec, receipts, requests)?)
    }
}

impl Variant for OptimismVariant {
    fn spec() -> ChainSpec {
        rsp_primitives::chain_spec::op_mainnet().unwrap()
    }

    fn execute<DB>(
        executor_block_input: &BlockWithSenders,
        executor_difficulty: U256,
        cache_db: DB,
    ) -> eyre::Result<BlockExecutionOutput<Receipt>>
    where
        DB: Database<Error: Into<ProviderError> + Display>,
    {
        Ok(OpExecutorProvider::new(Self::spec().into(), OptimismEvmConfig::default())
            .executor(cache_db)
            .execute((executor_block_input, executor_difficulty).into())?)
    }

    fn validate_block_post_execution(
        block: &BlockWithSenders,
        chain_spec: &ChainSpec,
        receipts: &[Receipt],
        _requests: &[Request],
    ) -> eyre::Result<()> {
        Ok(validate_block_post_execution_optimism(block, chain_spec, receipts)?)
    }
}
