#![feature(stmt_expr_attributes)]

pub mod io;
#[macro_use]
pub mod utils;

use eyre::eyre;
use io::GuestExecutorInput;
use reth_ethereum_consensus::validate_block_post_execution;
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_evm_ethereum::execute::EthExecutorProvider;
use reth_execution_types::ExecutionOutcome;
use reth_primitives::{proofs, Bloom, Header, Receipts};
use revm::db::CacheDB;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone, Default)]
pub struct GuestExecutor;

impl GuestExecutor {
    pub fn execute(&self, mut input: GuestExecutorInput) -> eyre::Result<Header> {
        // Initialize the witnessed database with verified storage proofs.
        let witness_db = input.witness_db()?;
        let cache_db = CacheDB::new(witness_db);

        // Execute the block.
        let spec = rsp_primitives::chain_spec::mainnet()?;
        let executor = EthExecutorProvider::ethereum(spec.clone().into()).executor(cache_db);
        let executor_block_input = profile!("recover senders", {
            input
                .current_block
                .clone()
                .with_recovered_senders()
                .ok_or(eyre!("failed to recover senders"))
        })?;
        let executor_difficulty = input.current_block.header.difficulty;
        let executor_output = profile!("execute", {
            executor.execute((&executor_block_input, executor_difficulty).into())
        })?;

        // Validate the block post execution.
        profile!("validate block post-execution", {
            validate_block_post_execution(
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
            rsp_mpt::compute_state_root(&executor_outcome, &input.dirty_storage_proofs)
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
