pub mod io;

use eyre::eyre;
use io::GuestExecutorInput;
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_evm_ethereum::execute::EthExecutorProvider;
use reth_execution_types::ExecutionOutcome;
use reth_primitives::Receipts;
use revm::db::CacheDB;

/// An executor that executes a block inside a zkVM.
#[derive(Debug, Clone, Default)]
pub struct GuestExecutor;

impl GuestExecutor {
    pub fn execute(&self, mut input: GuestExecutorInput) -> eyre::Result<()> {
        // Initialize the witnessed database with verified storage proofs.
        let witness_db = input.witness_db()?;
        let cache_db = CacheDB::new(witness_db);

        // Execute the block.
        let spec = rsp_primitives::chain_spec::mainnet()?;
        let executor = EthExecutorProvider::ethereum(spec.into()).executor(cache_db);
        let executor_block_input = input
            .current_block
            .clone()
            .with_recovered_senders()
            .ok_or(eyre!("failed to recover senders"))?;
        let executor_difficulty = input.current_block.header.difficulty;
        let executor_output =
            executor.execute((&executor_block_input, executor_difficulty).into())?;

        // Convert the output to an execution outcome.
        let executor_outcome = ExecutionOutcome::new(
            executor_output.state,
            Receipts::from(executor_output.receipts),
            input.current_block.header.number,
            vec![executor_output.requests.into()],
        );

        // Verify the state root.
        let state_root =
            rsp_mpt::compute_state_root(&executor_outcome, &input.dirty_storage_proofs)?;
        if state_root != input.current_block.state_root {
            eyre::bail!("mismatched state root");
        }
        println!("successfully verified state root");

        // TODO: Assert that the block header is correct.

        Ok(())
    }
}
