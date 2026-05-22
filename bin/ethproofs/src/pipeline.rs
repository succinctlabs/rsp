use alloy_provider::Provider;
use futures::{Stream, StreamExt};
use rsp_client_executor::io::ClientExecutorInput;
use rsp_host_executor::{
    alerting::AlertingClient, BlockExecutor, ExecutionHooks, ExecutorComponents, FullExecutor,
};
use sp1_sdk::SP1ProofMode;
use tokio::sync::mpsc;
use tracing::error;

/// Capacity of the channel connecting the fetch stage to the process stage.
///
/// Bounded so the fetch stage applies backpressure instead of running unboundedly ahead of
/// the (GPU-bound) prover, while still keeping the next block's input ready the moment the
/// prover frees up.
const CHANNEL_CAPACITY: usize = 2;

/// Drive blocks through a two-stage pipeline: fetch+witness -> process.
///
/// The two stages run concurrently over a bounded channel, so the next block's witness is
/// fetched while the current block is being processed (the fetch-ahead lever). Within the
/// process stage, the validation-execute and the proof generation for a block run
/// *concurrently* — proving does not depend on the execute, and they use different resources
/// (GPU vs CPU), so this keeps the validation-execute off the proving critical path.
///
/// Per-block errors are logged (and alerted) without tearing down the pipeline.
pub async fn run_pipeline<C, P>(
    executor: FullExecutor<C, P>,
    mut blocks: impl Stream<Item = u64> + Unpin,
    prove_mode: Option<SP1ProofMode>,
    alerting_client: Option<AlertingClient>,
) -> eyre::Result<()>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    let alerting = &alerting_client;
    let executor = &executor;

    let (fetch_tx, mut fetch_rx) =
        mpsc::channel::<(u64, ClientExecutorInput<C::Primitives>)>(CHANNEL_CAPACITY);

    // Stage 1: fetch the block and build its execution witness, running ahead of processing.
    let fetch = async move {
        while let Some(block_number) = blocks.next().await {
            let result = async {
                executor.wait_for_block(block_number).await?;
                executor.hooks().on_execution_start(block_number).await?;
                executor.fetch_client_input(block_number).await
            }
            .await;

            match result {
                Ok(input) => {
                    if fetch_tx.send((block_number, input)).await.is_err() {
                        break;
                    }
                }
                Err(err) => report(alerting, block_number, "fetch", err).await,
            }
        }
    };

    // Stage 2: process the block — validate-execute and (when proving) prove, concurrently.
    let process = async move {
        while let Some((block_number, input)) = fetch_rx.recv().await {
            if let Err(err) = process_block(executor, prove_mode, block_number, input).await {
                report(alerting, block_number, "process", err).await;
            }
        }
    };

    tokio::join!(fetch, process);

    Ok(())
}

/// Validate-execute a block and, when proving is enabled, prove it — running the two
/// concurrently so the execute (CPU) stays off the proving (GPU) critical path.
async fn process_block<C, P>(
    executor: &FullExecutor<C, P>,
    prove_mode: Option<SP1ProofMode>,
    block_number: u64,
    input: ClientExecutorInput<C::Primitives>,
) -> eyre::Result<()>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    let hooks = executor.hooks();
    let stdin = executor.build_stdin(&input)?;

    match prove_mode {
        // Proving enabled: run the validation-execute and proving concurrently.
        Some(prove_mode) => {
            let vk = executor.vk();
            let (cycles, proof) = tokio::join!(
                executor.execute_input(&input, &stdin, hooks),
                executor.prove_only(block_number, stdin.clone(), prove_mode, hooks),
            );
            let cycle_count = cycles?;
            let (proof_bytes, proving_duration) = proof?;

            hooks
                .on_proving_end(
                    block_number,
                    &proof_bytes,
                    vk.as_ref(),
                    cycle_count,
                    proving_duration,
                )
                .await?;
        }
        // Execute-only (proving disabled) — still runs for validation and metrics.
        None => {
            executor.execute_input(&input, &stdin, hooks).await?;
        }
    }

    Ok(())
}

async fn report(
    alerting: &Option<AlertingClient>,
    block_number: u64,
    stage: &str,
    err: eyre::Error,
) {
    let message = format!("Error handling block {block_number} at {stage} stage: {err}");
    error!(message);
    if let Some(alerting) = alerting {
        alerting.send_alert(message).await;
    }
}
