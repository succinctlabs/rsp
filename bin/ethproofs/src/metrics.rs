use std::{collections::HashSet, net::SocketAddr, sync::Mutex, time::Duration};

use alloy_consensus::{Block, BlockHeader};
use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use reth_primitives_traits::NodePrimitives;
use rsp_host_executor::ExecutionHooks;
use sp1_sdk::{ExecutionReport, SP1VerifyingKey};
use tracing::info;

/// Start a Prometheus exporter serving `/metrics` on the given address.
///
/// Returns an error if the listener cannot be installed (e.g. the port is already in use).
pub fn install_prometheus_exporter(addr: SocketAddr) -> eyre::Result<()> {
    PrometheusBuilder::new()
        .with_http_listener(addr)
        .install()
        .map_err(|err| eyre::eyre!("failed to install Prometheus exporter: {err}"))?;
    info!("Serving Prometheus metrics at http://{addr}/metrics");
    Ok(())
}

/// Record the chain head announced by the WS subscription. Together with
/// `rsp_ethproofs_last_proved_block` this lets dashboards compute how far the pipeline lags
/// behind the chain — the signal that proving is falling behind, before backlogged witness
/// fetches start failing.
pub fn record_chain_head(block_number: u64) {
    gauge!("rsp_ethproofs_chain_head_block").set(block_number as f64);
}

/// Record a sampled block at intake, before any fetch attempt — so intake monitoring keeps
/// counting even when every fetch fails (`blocks_seen_total >= blocks_failed_total` always
/// holds).
pub fn record_block_sampled() {
    counter!("rsp_ethproofs_blocks_seen_total").increment(1);
}

/// An [`ExecutionHooks`] implementation that records internal proving-service metrics.
///
/// Metrics are emitted through the `metrics` facade and surfaced by the Prometheus exporter
/// installed via [`install_prometheus_exporter`]. All metric names are prefixed `rsp_ethproofs_`.
#[derive(Debug, Default)]
pub struct MetricsHook {
    /// Blocks with proving in flight, so a failed block releases the in-progress gauge exactly
    /// once (via `on_block_failed`) even though `on_proving_end` never fires for it.
    proving_started: Mutex<HashSet<u64>>,
}

impl MetricsHook {
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear a block's proving-in-flight marker, decrementing the gauge if it was set.
    fn finish_proving(&self, block_number: u64) {
        if self.proving_started.lock().unwrap().remove(&block_number) {
            gauge!("rsp_ethproofs_proofs_in_progress").decrement(1.0);
        }
    }

    /// The full `on_proving_end` recording logic, split out (without the verifying key, which
    /// the metrics never use) so it is unit-testable — a real `SP1VerifyingKey` cannot be
    /// constructed in a test.
    fn record_proving_end(
        &self,
        block_number: u64,
        proof_size: usize,
        cycle_count: Option<u64>,
        proving_duration: Duration,
    ) {
        self.finish_proving(block_number);
        counter!("rsp_ethproofs_blocks_proved_total").increment(1);
        histogram!("rsp_ethproofs_proving_duration_seconds").record(proving_duration.as_secs_f64());
        histogram!("rsp_ethproofs_proof_size_bytes").record(proof_size as f64);

        // Proving throughput in kHz (cycles per second / 1000), the headline efficiency metric.
        if let Some(cycles) = cycle_count {
            let secs = proving_duration.as_secs_f64();
            if secs > 0.0 {
                gauge!("rsp_ethproofs_proving_khz").set((cycles as f64 / secs) / 1000.0);
            }
        }

        gauge!("rsp_ethproofs_last_proved_block").set(block_number as f64);
    }
}

impl ExecutionHooks for MetricsHook {
    async fn on_execution_start(&self, _block_number: u64) -> eyre::Result<()> {
        // Fired by the pipeline once a block's input is fetched and it enters the process
        // queue; intake counting (`blocks_seen_total`) happens earlier, at sampling time.
        counter!("rsp_ethproofs_blocks_queued_total").increment(1);
        Ok(())
    }

    async fn on_execution_end<P: NodePrimitives>(
        &self,
        executed_block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
        execution_duration: Duration,
    ) -> eyre::Result<()> {
        histogram!("rsp_ethproofs_execution_duration_seconds")
            .record(execution_duration.as_secs_f64());

        counter!("rsp_ethproofs_blocks_executed_total").increment(1);
        histogram!("rsp_ethproofs_cycles")
            .record(execution_report.total_instruction_count() as f64);
        // EVM gas used by the block.
        histogram!("rsp_ethproofs_gas_used").record(executed_block.header.gas_used() as f64);
        // SP1 gas (the prover's gas estimate), which tracks proving cost rather than EVM gas.
        histogram!("rsp_ethproofs_sp1_gas")
            .record(execution_report.gas().unwrap_or_default() as f64);
        histogram!("rsp_ethproofs_tx_count").record(executed_block.body.transactions.len() as f64);
        gauge!("rsp_ethproofs_last_executed_block").set(executed_block.number() as f64);

        Ok(())
    }

    async fn on_proving_start(&self, block_number: u64) -> eyre::Result<()> {
        if self.proving_started.lock().unwrap().insert(block_number) {
            gauge!("rsp_ethproofs_proofs_in_progress").increment(1.0);
        }
        Ok(())
    }

    async fn on_proving_end(
        &self,
        block_number: u64,
        proof_bytes: &[u8],
        _vk: &SP1VerifyingKey,
        cycle_count: Option<u64>,
        proving_duration: Duration,
    ) -> eyre::Result<()> {
        self.record_proving_end(block_number, proof_bytes.len(), cycle_count, proving_duration);

        Ok(())
    }

    async fn on_block_failed(&self, block_number: u64) -> eyre::Result<()> {
        counter!("rsp_ethproofs_blocks_failed_total").increment(1);
        self.finish_proving(block_number);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

    use super::*;

    /// The current value of a gauge in the snapshot, or 0.0 if it was never touched.
    fn gauge_value(snapshotter: &Snapshotter, name: &str) -> f64 {
        snapshotter
            .snapshot()
            .into_vec()
            .into_iter()
            .find_map(|(key, _, _, value)| {
                (key.key().name() == name).then(|| match value {
                    DebugValue::Gauge(value) => value.into_inner(),
                    _ => panic!("`{name}` is not a gauge"),
                })
            })
            .unwrap_or_default()
    }

    /// The proofs-in-progress gauge must be released exactly once per block, whether the block
    /// succeeds (`on_proving_end`) or fails (`on_block_failed`), and never go negative — the
    /// gauge previously drifted upward forever because failures skipped `on_proving_end`.
    #[tokio::test]
    async fn proofs_in_progress_gauge_is_released_exactly_once() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let hook = MetricsHook::new();
        let gauge = "rsp_ethproofs_proofs_in_progress";

        metrics::with_local_recorder(&recorder, || {
            futures::executor::block_on(async {
                // A successful block: starts, then ends.
                hook.on_proving_start(1).await.unwrap();
                assert_eq!(gauge_value(&snapshotter, gauge), 1.0);

                // A failing block: starts, then fails.
                hook.on_proving_start(2).await.unwrap();
                assert_eq!(gauge_value(&snapshotter, gauge), 2.0);

                hook.on_block_failed(2).await.unwrap();
                assert_eq!(gauge_value(&snapshotter, gauge), 1.0);

                // A repeated failure callback must not decrement twice.
                hook.on_block_failed(2).await.unwrap();
                assert_eq!(gauge_value(&snapshotter, gauge), 1.0);

                // Block 1 completes: the success path (the full recording logic behind
                // `on_proving_end`, minus the unused verifying key) must release the gauge.
                hook.record_proving_end(1, 1024, Some(1_000_000), Duration::from_secs(60));
                assert_eq!(gauge_value(&snapshotter, gauge), 0.0);

                // A failure for a block that never started proving (e.g. a fetch failure) must
                // not decrement the gauge either.
                hook.on_block_failed(3).await.unwrap();
                assert_eq!(gauge_value(&snapshotter, gauge), 0.0);
            })
        });
    }
}
