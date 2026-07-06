use std::{future::Future, sync::Arc, time::Duration};

use alloy_provider::Provider;
use eyre::eyre;
use futures::{Stream, StreamExt};
use rsp_client_executor::io::ClientExecutorInput;
use rsp_host_executor::{
    alerting::AlertingClient, BlockExecutor, ExecutionHooks, ExecutorComponents, FullExecutor,
};
use tokio::sync::mpsc;
use tracing::error;

/// Capacity of the channel connecting the fetch stage to the process stage.
///
/// Bounded so the fetch stage applies backpressure instead of running unboundedly ahead of
/// the (GPU-bound) prover, while still keeping the next block's input ready the moment the
/// prover frees up.
const CHANNEL_CAPACITY: usize = 2;

/// How long the fetch stage waits for a sampled block to appear on the RPC node before treating
/// it as a per-block failure (logged and alerted). Without this bound, a node that stops
/// syncing — while the WS node keeps announcing headers — would stall the pipeline forever
/// without ever alerting.
const WAIT_FOR_BLOCK_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Upper bound on the whole per-block fetch stage (waiting for the block, then fetching its
/// witness). The witness fetch runs many RPC round-trips over a client with no request timeout,
/// so without this bound a hung connection mid-fetch would stall the pipeline forever — the
/// same hazard [`WAIT_FOR_BLOCK_TIMEOUT`] covers for the wait half.
const FETCH_STAGE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The executor surface the pipeline drives, abstracted so the pipeline's orchestration
/// (fetch-ahead, per-block error tolerance, hook ordering) is testable without an RPC node or
/// a prover.
pub trait PipelineExecutor {
    /// The fetched per-block input handed from the fetch stage to the process stage.
    type Input: Send;

    /// Wait until the block is available on the data source.
    fn wait_for_block(&self, block_number: u64) -> impl Future<Output = eyre::Result<()>>;

    /// Fetch the block's input (its execution witness).
    fn fetch_input(&self, block_number: u64) -> impl Future<Output = eyre::Result<Self::Input>>;

    /// Report a block as queued. Fired only once its input is in hand, so a failed fetch does
    /// not leave a permanently-"queued" proof on the ethproofs dashboard.
    fn on_queued(&self, block_number: u64) -> impl Future<Output = eyre::Result<()>>;

    /// Validate-execute the block and, when proving is enabled, prove it.
    fn process(
        &self,
        block_number: u64,
        input: Self::Input,
    ) -> impl Future<Output = eyre::Result<()>>;

    /// Release per-block state after a failure at any stage.
    fn on_failed(&self, block_number: u64) -> impl Future<Output = eyre::Result<()>>;
}

/// Drive blocks through a two-stage pipeline: fetch+witness -> process.
///
/// The two stages run concurrently over a bounded channel, so the next block's witness is
/// fetched while the current block is being processed (the fetch-ahead lever).
///
/// Per-block errors are logged (and alerted) without tearing down the pipeline.
pub async fn run_pipeline<E>(
    executor: E,
    mut blocks: impl Stream<Item = u64> + Unpin,
    alerting_client: Option<Arc<AlertingClient>>,
) where
    E: PipelineExecutor,
{
    let alerting = &alerting_client;
    let executor = &executor;

    let (fetch_tx, mut fetch_rx) = mpsc::channel::<(u64, E::Input)>(CHANNEL_CAPACITY);

    // Stage 1: fetch the block and build its execution witness, running ahead of processing.
    let fetch = async move {
        while let Some(block_number) = blocks.next().await {
            let fetch_one = async {
                tokio::time::timeout(WAIT_FOR_BLOCK_TIMEOUT, executor.wait_for_block(block_number))
                    .await
                    .map_err(|_| {
                        eyre!(
                            "timed out after {WAIT_FOR_BLOCK_TIMEOUT:?} waiting for the block to \
                         appear on the RPC node"
                        )
                    })??;

                let input = executor.fetch_input(block_number).await?;

                executor.on_queued(block_number).await?;

                eyre::Ok(input)
            };

            let result = match tokio::time::timeout(FETCH_STAGE_TIMEOUT, fetch_one).await {
                Ok(result) => result,
                Err(_) => Err(eyre!(
                    "fetch stage timed out after {FETCH_STAGE_TIMEOUT:?} (the witness fetch \
                     likely hung on a dead RPC connection)"
                )),
            };

            match result {
                Ok(input) => {
                    if fetch_tx.send((block_number, input)).await.is_err() {
                        break;
                    }
                }
                Err(err) => report(executor, alerting, block_number, "fetch", err).await,
            }
        }
    };

    // Stage 2: process the block — validate-execute and (when proving) prove, concurrently.
    let process = async move {
        while let Some((block_number, input)) = fetch_rx.recv().await {
            if let Err(err) = executor.process(block_number, input).await {
                report(executor, alerting, block_number, "process", err).await;
            }
        }
    };

    tokio::join!(fetch, process);
}

impl<C, P> PipelineExecutor for FullExecutor<C, P>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    type Input = ClientExecutorInput<C::Primitives>;

    async fn wait_for_block(&self, block_number: u64) -> eyre::Result<()> {
        FullExecutor::wait_for_block(self, block_number).await
    }

    async fn fetch_input(&self, block_number: u64) -> eyre::Result<Self::Input> {
        self.fetch_client_input(block_number).await
    }

    async fn on_queued(&self, block_number: u64) -> eyre::Result<()> {
        self.hooks().on_execution_start(block_number).await
    }

    async fn process(&self, _block_number: u64, input: Self::Input) -> eyre::Result<()> {
        self.process_client_concurrent(input, self.hooks()).await
    }

    async fn on_failed(&self, block_number: u64) -> eyre::Result<()> {
        self.hooks().on_block_failed(block_number).await
    }
}

/// Log and alert a per-block failure, and fire the executor's failure callback so hooks can
/// release any per-block state (e.g. the proofs-in-progress gauge).
///
/// The alert is awaited (bounded by the alerting client's request timeout) rather than
/// detached, so an alert in flight when the pipeline shuts down is still delivered.
async fn report<E: PipelineExecutor>(
    executor: &E,
    alerting: &Option<Arc<AlertingClient>>,
    block_number: u64,
    stage: &str,
    err: eyre::Error,
) {
    let message = format!("Error handling block {block_number} at {stage} stage: {err}");
    error!(message);

    if let Err(hook_err) = executor.on_failed(block_number).await {
        error!("on_failed hook failed for block {block_number}: {hook_err}");
    }

    if let Some(alerting) = alerting {
        alerting.send_alert(message).await;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };

    use eyre::bail;
    use futures::stream;

    use super::*;

    /// Records the pipeline's calls in order; individual blocks can be made to fail or stall.
    #[derive(Debug, Default)]
    struct MockExecutor {
        events: Arc<Mutex<Vec<String>>>,
        stall_wait: HashSet<u64>,
        stall_fetch: HashSet<u64>,
        fail_fetch: HashSet<u64>,
        fail_process: HashSet<u64>,
        process_delay: Option<Duration>,
    }

    impl MockExecutor {
        fn log(&self, event: String) {
            self.events.lock().unwrap().push(event);
        }
    }

    impl PipelineExecutor for MockExecutor {
        type Input = u64;

        async fn wait_for_block(&self, block_number: u64) -> eyre::Result<()> {
            if self.stall_wait.contains(&block_number) {
                // A node that never serves the block; only the pipeline's timeout ends this.
                std::future::pending::<()>().await;
            }
            Ok(())
        }

        async fn fetch_input(&self, block_number: u64) -> eyre::Result<u64> {
            if self.stall_fetch.contains(&block_number) {
                // A witness fetch hanging on a dead connection; only the stage timeout ends it.
                std::future::pending::<()>().await;
            }
            if self.fail_fetch.contains(&block_number) {
                bail!("fetch failed");
            }
            self.log(format!("fetch:{block_number}"));
            Ok(block_number)
        }

        async fn on_queued(&self, block_number: u64) -> eyre::Result<()> {
            self.log(format!("queued:{block_number}"));
            Ok(())
        }

        async fn process(&self, block_number: u64, _input: u64) -> eyre::Result<()> {
            self.log(format!("process_start:{block_number}"));
            if let Some(delay) = self.process_delay {
                tokio::time::sleep(delay).await;
            }
            if self.fail_process.contains(&block_number) {
                bail!("process failed");
            }
            self.log(format!("process_end:{block_number}"));
            Ok(())
        }

        async fn on_failed(&self, block_number: u64) -> eyre::Result<()> {
            self.log(format!("failed:{block_number}"));
            Ok(())
        }
    }

    fn index_of(events: &[String], event: &str) -> usize {
        events.iter().position(|e| e == event).unwrap_or_else(|| {
            panic!("event `{event}` not found in {events:?}");
        })
    }

    #[tokio::test]
    async fn per_block_failures_do_not_tear_down_the_pipeline() {
        let executor =
            MockExecutor { fail_fetch: [2].into(), fail_process: [3].into(), ..Default::default() };
        let events = executor.events.clone();

        run_pipeline(executor, stream::iter([1, 2, 3, 4]), None).await;

        let events = events.lock().unwrap();

        // Blocks 1 and 4 complete despite the failures in between.
        assert!(events.contains(&"process_end:1".to_string()));
        assert!(events.contains(&"process_end:4".to_string()));

        // Failed blocks fire the failure callback (releasing hook state) and nothing else after.
        assert!(events.contains(&"failed:2".to_string()));
        assert!(events.contains(&"failed:3".to_string()));
        assert!(!events.contains(&"process_end:3".to_string()));
    }

    #[tokio::test]
    async fn queued_fires_only_after_a_successful_fetch() {
        let executor = MockExecutor { fail_fetch: [2].into(), ..Default::default() };
        let events = executor.events.clone();

        run_pipeline(executor, stream::iter([1, 2]), None).await;

        let events = events.lock().unwrap();

        // A failed fetch must not report the block as queued to ethproofs.
        assert!(!events.contains(&"queued:2".to_string()));

        // And a successful fetch reports queued before processing starts.
        assert!(index_of(&events, "queued:1") < index_of(&events, "process_start:1"));
    }

    #[tokio::test(start_paused = true)]
    async fn fetch_runs_ahead_while_a_block_is_processing() {
        let executor =
            MockExecutor { process_delay: Some(Duration::from_secs(60)), ..Default::default() };
        let events = executor.events.clone();

        run_pipeline(executor, stream::iter([1, 2, 3, 4]), None).await;

        let events = events.lock().unwrap();

        // While block 1 spends a minute processing, the fetch stage pre-fetches the following
        // blocks instead of idling — the whole point of the two-stage pipeline. Block 4 is the
        // sensitive probe: its fetch starts this early only when the channel really buffers
        // CHANNEL_CAPACITY (2) inputs ahead; with a buffer of 1 it would only be fetched after
        // block 1 finishes processing.
        assert!(index_of(&events, "fetch:4") < index_of(&events, "process_end:1"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_node_times_out_instead_of_hanging_the_pipeline() {
        let executor = MockExecutor { stall_wait: [1].into(), ..Default::default() };
        let events = executor.events.clone();

        run_pipeline(executor, stream::iter([1, 2]), None).await;

        let events = events.lock().unwrap();

        // Block 1 hits the wait timeout and is reported failed; block 2 still goes through.
        assert!(events.contains(&"failed:1".to_string()));
        assert!(events.contains(&"process_end:2".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_hung_witness_fetch_times_out_instead_of_hanging_the_pipeline() {
        let executor = MockExecutor { stall_fetch: [1].into(), ..Default::default() };
        let events = executor.events.clone();

        run_pipeline(executor, stream::iter([1, 2]), None).await;

        let events = events.lock().unwrap();

        // Block 1's witness fetch hangs, hits the stage timeout, and is reported failed —
        // without ever being reported as queued — while block 2 still goes through.
        assert!(events.contains(&"failed:1".to_string()));
        assert!(!events.contains(&"queued:1".to_string()));
        assert!(events.contains(&"process_end:2".to_string()));
    }
}
