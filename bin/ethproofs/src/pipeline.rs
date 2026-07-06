use std::sync::Arc;

use alloy_provider::Provider;
use eyre::bail;
use futures::{Stream, StreamExt};
use rsp_client_executor::io::ClientExecutorInput;
use rsp_host_executor::{
    alerting::AlertingClient, BlockExecutor, ExecutionHooks, ExecutorComponents, FullExecutor,
};
use tokio::sync::mpsc;
use tracing::{error, warn};

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
/// Whether blocks are proved or only executed is read from the executor's own config
/// (`prove_mode`), so there is a single source of truth for that setting.
///
/// Per-block errors are logged (and alerted) without tearing down the pipeline.
pub async fn run_pipeline<C, P>(
    executor: FullExecutor<C, P>,
    mut blocks: impl Stream<Item = u64> + Unpin,
    alerting_client: Option<AlertingClient>,
) where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    let alerting = &alerting_client.map(Arc::new);
    let executor = &executor;

    let (fetch_tx, mut fetch_rx) =
        mpsc::channel::<(u64, ClientExecutorInput<C::Primitives>)>(CHANNEL_CAPACITY);

    // Stage 1: fetch the block and build its execution witness, running ahead of processing.
    let fetch = async move {
        while let Some(block_number) = blocks.next().await {
            let result = async {
                executor.wait_for_block(block_number).await?;
                let input = executor.fetch_client_input(block_number).await?;

                // Report the block (queued to ethproofs, seen by metrics) only once its input
                // is actually in hand, so a failed fetch doesn't leave a permanently-"queued"
                // proof on the ethproofs dashboard.
                executor.hooks().on_execution_start(block_number).await?;

                eyre::Ok(input)
            }
            .await;

            match result {
                Ok(input) => {
                    if fetch_tx.send((block_number, input)).await.is_err() {
                        break;
                    }
                }
                Err(err) => report(executor.hooks(), alerting, block_number, "fetch", err).await,
            }
        }
    };

    // Stage 2: process the block — validate-execute and (when proving) prove, concurrently.
    let process = async move {
        while let Some((block_number, input)) = fetch_rx.recv().await {
            if let Err(err) = process_block(executor, block_number, input).await {
                report(executor.hooks(), alerting, block_number, "process", err).await;
            }
        }
    };

    tokio::join!(fetch, process);
}

/// Validate-execute a block and, when proving is enabled, prove it — running the two
/// concurrently so the execute (CPU) stays off the proving (GPU) critical path.
async fn process_block<C, P>(
    executor: &FullExecutor<C, P>,
    block_number: u64,
    input: ClientExecutorInput<C::Primitives>,
) -> eyre::Result<()>
where
    C: ExecutorComponents,
    P: Provider<C::Network> + Clone + std::fmt::Debug,
{
    let hooks = executor.hooks();
    let stdin = executor.build_stdin(&input)?;

    match executor.config().prove_mode {
        // Proving enabled: run the validation-execute and proving concurrently.
        Some(prove_mode) => {
            let vk = executor.vk();
            let (execution, proof) = tokio::join!(
                executor.execute_input(&input, stdin.clone(), hooks),
                executor.prove_only(block_number, stdin, prove_mode, hooks),
            );

            match (execution, proof) {
                (Ok(cycle_count), Ok((proof_bytes, proving_duration))) => {
                    hooks
                        .on_proving_end(
                            block_number,
                            &proof_bytes,
                            vk.as_ref(),
                            Some(cycle_count),
                            proving_duration,
                        )
                        .await?;
                }
                // At least one side failed: surface every failure instead of letting one
                // error mask the other.
                (execution, proof) => {
                    let mut errors = Vec::new();

                    if let Err(err) = execution {
                        errors.push(format!("execution failed: {err}"));
                    }
                    match proof {
                        Err(err) => errors.push(format!("proving failed: {err}")),
                        // The proof completed but the execution didn't, so the cycle count
                        // required for submission is missing and the proof cannot be used.
                        Ok(_) => warn!(
                            block_number,
                            "discarding a completed proof because the validation-execute failed"
                        ),
                    }

                    bail!(errors.join("; "));
                }
            }
        }
        // Execute-only (proving disabled) — still runs for validation and metrics.
        None => {
            executor.execute_input(&input, stdin, hooks).await?;
        }
    }

    Ok(())
}

/// Log and alert a per-block failure, and fire `on_block_failed` so hooks can release any
/// per-block state (e.g. the proofs-in-progress gauge).
async fn report<H: ExecutionHooks>(
    hooks: &H,
    alerting: &Option<Arc<AlertingClient>>,
    block_number: u64,
    stage: &str,
    err: eyre::Error,
) {
    let message = format!("Error handling block {block_number} at {stage} stage: {err}");
    error!(message);

    if let Err(hook_err) = hooks.on_block_failed(block_number).await {
        error!("on_block_failed hook failed for block {block_number}: {hook_err}");
    }

    // Deliver the alert in a detached task so a slow PagerDuty never stalls the pipeline.
    if let Some(alerting) = alerting {
        let alerting = alerting.clone();
        tokio::spawn(async move { alerting.send_alert(message).await });
    }
}
