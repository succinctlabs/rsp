use crate::custom::OpCodeTrackingInspector;
use alloy_consensus::BlockHeader;
use alloy_eips::eip7928::compute_block_access_list_hash;
use alloy_primitives::B256;
use evm2::evm::{Database, Db};
use reth_errors::BlockExecutionError;
use reth_evm::{BlockExecutor, BlockExecutorFactory, ConfigureEvm, Evm};
use reth_execution_types::BlockExecutionOutput;
use reth_primitives_traits::{BlockTy, ReceiptTy, RecoveredBlock};

type OutputWithBalHash<R> = (BlockExecutionOutput<R>, Option<B256>);

/// Execute a single witnessed block on the guest, including the pre- and post-block system calls.
pub(crate) fn execute_block<C: ConfigureEvm>(
    config: &C,
    factory: &C::BlockExecutorFactory,
    db: impl Database,
    block: &RecoveredBlock<BlockTy<C::Primitives>>,
    track_opcodes: bool,
) -> Result<OutputWithBalHash<ReceiptTy<C::Primitives>>, BlockExecutionError> {
    let env = config.evm_env(block.header()).map_err(BlockExecutionError::other)?;
    let ctx = config.context_for_block(block.sealed_block()).map_err(BlockExecutionError::other)?;
    let mut evm = factory.evm_with_env(Db::new(db), env);
    if track_opcodes {
        evm.set_inspector(OpCodeTrackingInspector::default());
    }
    let mut executor = factory.create_executor(evm, ctx);
    if block.header().block_access_list_hash().is_some() {
        executor.enable_block_access_list_builder();
    }
    executor.apply_pre_execution_changes()?;
    for tx in block.transactions_recovered() {
        executor.execute_transaction(tx)?;
    }
    let (output, bal) = executor.finish_with_block_access_list()?;
    Ok((output, bal.as_deref().map(compute_block_access_list_hash)))
}
