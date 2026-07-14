//! An [`ExecutionHooks`] that accumulates execution/proving timing across a run and logs an
//! aggregate summary on demand (at shutdown, or after `--max-blocks`).
//!
//! This complements the Prometheus histograms in [`crate::metrics`] — which already expose
//! per-block proving-time distributions to a scraper — with a log-visible summary that needs no
//! Prometheus. It keeps only running aggregates (count/sum/min/max), never a per-block vector, so
//! it is safe for an indefinitely-running service.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use alloy_consensus::Block;
use reth_primitives_traits::NodePrimitives;
use rsp_host_executor::ExecutionHooks;
use sp1_sdk::{ExecutionReport, SP1VerifyingKey};
use tracing::info;

/// Running statistics for a series of durations, without storing the individual samples.
#[derive(Debug, Default)]
struct DurationStats {
    count: u64,
    sum: Duration,
    min: Option<Duration>,
    max: Option<Duration>,
}

impl DurationStats {
    fn record(&mut self, d: Duration) {
        self.count += 1;
        self.sum += d;
        self.min = Some(self.min.map_or(d, |m| m.min(d)));
        self.max = Some(self.max.map_or(d, |m| m.max(d)));
    }

    fn mean(&self) -> Duration {
        // `sum` is zero when nothing was recorded, so returning it covers the empty case (and
        // avoids dividing by zero); `u32` is ample for any realistic block count.
        match u32::try_from(self.count) {
            Ok(n) if n > 0 => self.sum / n,
            _ => self.sum,
        }
    }
}

#[derive(Debug, Default)]
struct Acc {
    execution: DurationStats,
    proving: DurationStats,
    total_cycles: u128,
}

/// Accumulates timing across a run and logs a summary via [`Self::log_summary`]. Cheap to clone
/// (the state is shared behind an `Arc`), so a handle can be kept for the summary call while the
/// hook itself is moved into the executor.
#[derive(Debug, Clone, Default)]
pub struct ProvingSummary {
    acc: Arc<Mutex<Acc>>,
}

impl ProvingSummary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a completed execution. Split out from the hook so it is unit-testable.
    fn record_execution(&self, duration: Duration, cycles: u64) {
        let mut acc = self.acc.lock().unwrap();
        acc.execution.record(duration);
        acc.total_cycles += cycles as u128;
    }

    /// Record a completed proof. Split out from the hook (which also carries the verifying key,
    /// which cannot be constructed in a test) so it is unit-testable.
    fn record_proving(&self, duration: Duration) {
        self.acc.lock().unwrap().proving.record(duration);
    }

    /// Log an aggregate summary of everything recorded so far. A no-op (beyond a note) when
    /// nothing completed — e.g. a run that failed every block.
    pub fn log_summary(&self) {
        let acc = self.acc.lock().unwrap();

        if acc.execution.count == 0 && acc.proving.count == 0 {
            info!("Run summary: no blocks completed");
            return;
        }

        let e = &acc.execution;
        let p = &acc.proving;
        info!(
            "Run summary: {} executed (mean {:?}, min {:?}, max {:?}), \
             {} proved (mean {:?}, min {:?}, max {:?}, total {:?}), {} total cycles",
            e.count,
            e.mean(),
            e.min.unwrap_or_default(),
            e.max.unwrap_or_default(),
            p.count,
            p.mean(),
            p.min.unwrap_or_default(),
            p.max.unwrap_or_default(),
            p.sum,
            acc.total_cycles,
        );
    }
}

impl ExecutionHooks for ProvingSummary {
    async fn on_execution_end<P: NodePrimitives>(
        &self,
        _executed_block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
        execution_duration: Duration,
    ) -> eyre::Result<()> {
        self.record_execution(execution_duration, execution_report.total_instruction_count());
        Ok(())
    }

    async fn on_proving_end(
        &self,
        _block_number: u64,
        _proof_bytes: &[u8],
        _vk: &SP1VerifyingKey,
        _cycle_count: Option<u64>,
        proving_duration: Duration,
    ) -> eyre::Result<()> {
        self.record_proving(proving_duration);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_stats_track_count_mean_min_max() {
        let mut s = DurationStats::default();
        assert_eq!(s.mean(), Duration::ZERO);

        s.record(Duration::from_secs(10));
        s.record(Duration::from_secs(20));
        s.record(Duration::from_secs(30));

        assert_eq!(s.count, 3);
        assert_eq!(s.mean(), Duration::from_secs(20));
        assert_eq!(s.min, Some(Duration::from_secs(10)));
        assert_eq!(s.max, Some(Duration::from_secs(30)));
    }

    #[test]
    fn records_feed_the_right_series() {
        let summary = ProvingSummary::new();
        summary.record_execution(Duration::from_secs(2), 1_000);
        summary.record_proving(Duration::from_secs(40));
        summary.record_proving(Duration::from_secs(60));

        // Must not panic with data present.
        summary.log_summary();

        let acc = summary.acc.lock().unwrap();
        assert_eq!(acc.execution.count, 1);
        assert_eq!(acc.total_cycles, 1_000);
        assert_eq!(acc.proving.count, 2);
        assert_eq!(acc.proving.mean(), Duration::from_secs(50));
    }
}
