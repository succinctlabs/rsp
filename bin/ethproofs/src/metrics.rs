use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

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

/// An [`ExecutionHooks`] implementation that records internal proving-service metrics.
///
/// Metrics are emitted through the `metrics` facade and surfaced by the Prometheus exporter
/// installed via [`install_prometheus_exporter`]. All metric names are prefixed `rsp_ethproofs_`.
#[derive(Debug, Default)]
pub struct MetricsHook {
    /// Wall-clock start of execution per in-flight block, used to measure execution latency.
    execution_started: Mutex<HashMap<u64, Instant>>,
}

impl MetricsHook {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ExecutionHooks for MetricsHook {
    async fn on_execution_start(&self, block_number: u64) -> eyre::Result<()> {
        self.execution_started.lock().unwrap().insert(block_number, Instant::now());
        counter!("rsp_ethproofs_blocks_seen_total").increment(1);
        Ok(())
    }

    async fn on_execution_end<P: NodePrimitives>(
        &self,
        executed_block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        let block_number = executed_block.number();

        if let Some(started) = self.execution_started.lock().unwrap().remove(&block_number) {
            histogram!("rsp_ethproofs_execution_duration_seconds")
                .record(started.elapsed().as_secs_f64());
        }

        counter!("rsp_ethproofs_blocks_executed_total").increment(1);
        histogram!("rsp_ethproofs_cycles")
            .record(execution_report.total_instruction_count() as f64);
        // EVM gas used by the block.
        histogram!("rsp_ethproofs_gas_used").record(executed_block.header.gas_used() as f64);
        // SP1 gas (the prover's gas estimate), which tracks proving cost rather than EVM gas.
        histogram!("rsp_ethproofs_sp1_gas")
            .record(execution_report.gas().unwrap_or_default() as f64);
        histogram!("rsp_ethproofs_tx_count").record(executed_block.body.transactions.len() as f64);
        gauge!("rsp_ethproofs_last_executed_block").set(block_number as f64);

        Ok(())
    }

    async fn on_proving_start(&self, _block_number: u64) -> eyre::Result<()> {
        gauge!("rsp_ethproofs_proofs_in_progress").increment(1.0);
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
        gauge!("rsp_ethproofs_proofs_in_progress").decrement(1.0);
        counter!("rsp_ethproofs_blocks_proved_total").increment(1);
        histogram!("rsp_ethproofs_proving_duration_seconds").record(proving_duration.as_secs_f64());
        histogram!("rsp_ethproofs_proof_size_bytes").record(proof_bytes.len() as f64);

        // Proving throughput in kHz (cycles per second / 1000), the headline efficiency metric.
        if let Some(cycles) = cycle_count {
            let secs = proving_duration.as_secs_f64();
            if secs > 0.0 {
                gauge!("rsp_ethproofs_proving_khz").set((cycles as f64 / secs) / 1000.0);
            }
        }

        gauge!("rsp_ethproofs_last_proved_block").set(block_number as f64);

        Ok(())
    }
}
