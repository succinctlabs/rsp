use alloy_evm::Database;
use reth_errors::BlockExecutionError;
use reth_evm::{
    execute::{BlockExecutionStrategy, BlockExecutionStrategyFactory, Executor},
    system_calls::OnStateHook,
};
use reth_execution_types::BlockExecutionResult;
use reth_primitives::{NodePrimitives, RecoveredBlock};
use revm::database::{states::bundle_state::BundleRetention, State};

use crate::custom::OpCodeTrackingInspector;

/// A generic block executor that uses a [`BlockExecutionStrategy`] to
/// execute blocks.
#[allow(missing_debug_implementations, dead_code)]
pub struct OpCodesTrackingBlockExecutor<F, DB> {
    /// Block execution strategy.
    pub(crate) strategy_factory: F,
    /// Database.
    pub(crate) db: State<DB>,
}

impl<F, DB: Database> OpCodesTrackingBlockExecutor<F, DB> {
    /// Creates a new `CustomBlockExecutor` with the given strategy.
    pub fn new(strategy_factory: F, db: DB) -> Self {
        let db =
            State::builder().with_database(db).with_bundle_update().without_state_clear().build();
        Self { strategy_factory, db }
    }
}

impl<F, DB> Executor<DB> for OpCodesTrackingBlockExecutor<F, DB>
where
    F: BlockExecutionStrategyFactory,
    DB: Database,
{
    type Primitives = F::Primitives;
    type Error = BlockExecutionError;

    fn execute_one(
        &mut self,
        block: &RecoveredBlock<<Self::Primitives as NodePrimitives>::Block>,
    ) -> Result<BlockExecutionResult<<Self::Primitives as NodePrimitives>::Receipt>, Self::Error>
    {
        let evm_env = self.strategy_factory.evm_env(block.header());
        let evm = self.strategy_factory.evm_with_env_and_inspector(
            &mut self.db,
            evm_env,
            OpCodeTrackingInspector::default(),
        );
        let ctx = self.strategy_factory.context_for_block(block);
        let mut strategy = self.strategy_factory.create_strategy(evm, ctx);

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
        let mut strategy = self.strategy_factory.strategy_for_block(&mut self.db, block);
        strategy.with_state_hook(Some(Box::new(state_hook)));

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
