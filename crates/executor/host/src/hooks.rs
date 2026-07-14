use std::{future::Future, time::Duration};

use alloy_consensus::Block;
use reth_primitives_traits::NodePrimitives;
use sp1_sdk::{ExecutionReport, SP1VerifyingKey};

pub trait ExecutionHooks: Send {
    fn on_execution_start(
        &self,
        _block_number: u64,
    ) -> impl Future<Output = eyre::Result<()>> + Send {
        async { Ok(()) }
    }

    fn on_execution_end<P: NodePrimitives>(
        &self,
        _executed_block: &Block<P::SignedTx>,
        _execution_report: &ExecutionReport,
        _execution_duration: Duration,
    ) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }

    fn on_proving_start(&self, _block_number: u64) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }

    /// Called when a block fails at any stage, so hooks can release per-block state (e.g.
    /// in-progress gauges) that the success callbacks would otherwise have cleaned up.
    fn on_block_failed(&self, _block_number: u64) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }

    fn on_proving_end(
        &self,
        _block_number: u64,
        _proof_bytes: &[u8],
        _vk: &SP1VerifyingKey,
        _cycle_count: Option<u64>,
        _proving_duration: Duration,
    ) -> impl Future<Output = eyre::Result<()>> {
        async { Ok(()) }
    }
}

impl ExecutionHooks for () {}

/// Combine the results of two independent hooks, surfacing both errors when both fail.
fn combine(first: eyre::Result<()>, second: eyre::Result<()>) -> eyre::Result<()> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(err), Ok(())) | (Ok(()), Err(err)) => Err(err),
        (Err(first), Err(second)) => Err(eyre::eyre!("{first}; {second}")),
    }
}

/// Fans out every hook callback to a pair of hooks, in order.
///
/// This lets a binary run several independent hooks at once (e.g. an ethproofs reporter
/// alongside a metrics collector). Nest tuples to compose more than two.
///
/// The hooks are independent, so both are always invoked even when the first one fails —
/// otherwise one hook's error would silently skip the other's bookkeeping (e.g. releasing an
/// in-progress gauge). Errors from both are surfaced.
impl<A, B> ExecutionHooks for (A, B)
where
    A: ExecutionHooks + Sync,
    B: ExecutionHooks + Sync,
{
    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        combine(
            self.0.on_execution_start(block_number).await,
            self.1.on_execution_start(block_number).await,
        )
    }

    async fn on_execution_end<P: NodePrimitives>(
        &self,
        executed_block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
        execution_duration: Duration,
    ) -> eyre::Result<()> {
        combine(
            self.0
                .on_execution_end::<P>(executed_block, execution_report, execution_duration)
                .await,
            self.1
                .on_execution_end::<P>(executed_block, execution_report, execution_duration)
                .await,
        )
    }

    async fn on_proving_start(&self, block_number: u64) -> eyre::Result<()> {
        combine(
            self.0.on_proving_start(block_number).await,
            self.1.on_proving_start(block_number).await,
        )
    }

    async fn on_block_failed(&self, block_number: u64) -> eyre::Result<()> {
        combine(
            self.0.on_block_failed(block_number).await,
            self.1.on_block_failed(block_number).await,
        )
    }

    async fn on_proving_end(
        &self,
        block_number: u64,
        proof_bytes: &[u8],
        vk: &SP1VerifyingKey,
        cycle_count: Option<u64>,
        proving_duration: Duration,
    ) -> eyre::Result<()> {
        combine(
            self.0
                .on_proving_end(block_number, proof_bytes, vk, cycle_count, proving_duration)
                .await,
            self.1
                .on_proving_end(block_number, proof_bytes, vk, cycle_count, proving_duration)
                .await,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A hook that always fails its callbacks.
    struct FailingHook;

    impl ExecutionHooks for FailingHook {
        async fn on_block_failed(&self, _block_number: u64) -> eyre::Result<()> {
            eyre::bail!("failing hook")
        }
    }

    /// A hook that counts its callbacks.
    #[derive(Default)]
    struct CountingHook {
        calls: AtomicUsize,
    }

    impl ExecutionHooks for CountingHook {
        async fn on_block_failed(&self, _block_number: u64) -> eyre::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// The second hook must run even when the first one fails, or one hook's error would
    /// silently skip the other's per-block cleanup (e.g. releasing an in-progress gauge).
    #[tokio::test]
    async fn tuple_fan_out_runs_both_hooks_even_when_the_first_fails() {
        let hooks = (FailingHook, CountingHook::default());

        let result = hooks.on_block_failed(1).await;

        assert!(result.is_err());
        assert_eq!(hooks.1.calls.load(Ordering::SeqCst), 1);
    }
}
