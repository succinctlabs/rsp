use alloy_evm::Database;
use reth_errors::BlockExecutionError;
use reth_evm::{block::BlockExecutor, execute::Executor, ConfigureEvm, OnStateHook};
use reth_execution_types::BlockExecutionResult;
use reth_primitives_traits::{NodePrimitives, RecoveredBlock};
use revm::database::{states::bundle_state::BundleRetention, State};

use crate::custom::OpCodeTrackingInspector;

/// A generic block executor that uses a [`BlockExecutionStrategy`] to
/// execute blocks.
#[allow(missing_debug_implementations, dead_code)]
pub struct OpCodesTrackingBlockExecutor<C, DB> {
    /// EVM config.
    pub(crate) evm_config: C,
    /// Database.
    pub(crate) db: State<DB>,
}

impl<C, DB: Database> OpCodesTrackingBlockExecutor<C, DB> {
    /// Creates a new `CustomBlockExecutor` with the given strategy.
    pub fn new(evm_config: C, db: DB) -> Self {
        let db =
            State::builder().with_database(db).with_bundle_update().without_state_clear().build();
        Self { evm_config, db }
    }
}

impl<C, DB> Executor<DB> for OpCodesTrackingBlockExecutor<C, DB>
where
    C: ConfigureEvm,
    DB: Database,
{
    type Primitives = C::Primitives;
    type Error = BlockExecutionError;

    fn execute_one(
        &mut self,
        block: &RecoveredBlock<<Self::Primitives as NodePrimitives>::Block>,
    ) -> Result<BlockExecutionResult<<Self::Primitives as NodePrimitives>::Receipt>, Self::Error>
    {
        let evm_env = self.evm_config.evm_env(block.header());
        let evm = self.evm_config.evm_with_env_and_inspector(
            &mut self.db,
            evm_env,
            OpCodeTrackingInspector::default(),
        );
        let ctx = self.evm_config.context_for_block(block);
        let mut strategy = self.evm_config.create_executor(evm, ctx);

        strategy.apply_pre_execution_changes()?;
        for tx in block.transactions_recovered() {
            strategy.execute_transaction(tx)?;
        }
        let result = strategy.apply_post_execution_changes()?;

        self.db.merge_transitions(BundleRetention::Reverts);

        Ok(result)
    }

    fn execute_one_with_state_hook<H>(
        &mut self,
        block: &RecoveredBlock<<Self::Primitives as NodePrimitives>::Block>,
        state_hook: H,
    ) -> Result<BlockExecutionResult<<Self::Primitives as NodePrimitives>::Receipt>, Self::Error>
    where
        H: OnStateHook + 'static,
    {
        let mut strategy = self
            .evm_config
            .executor_for_block(&mut self.db, block)
            .with_state_hook(Some(Box::new(state_hook)));

        strategy.apply_pre_execution_changes()?;
        for tx in block.transactions_recovered() {
            strategy.execute_transaction(tx)?;
        }
        let result = strategy.apply_post_execution_changes()?;

        self.db.merge_transitions(BundleRetention::Reverts);

        Ok(result)
    }

    fn into_state(self) -> State<DB> {
        self.db
    }

    fn size_hint(&self) -> usize {
        self.db.bundle_state.size_hint()
    }
}
