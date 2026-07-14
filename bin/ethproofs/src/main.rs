use std::{sync::Arc, time::Duration};

use alloy_provider::{Provider, ProviderBuilder, WsConnect};
use clap::Parser;
use cli::Args;
use ethproofs::EthproofsClient;
use eyre::bail;
use metrics::{install_prometheus_exporter, MetricsHook};
use pipeline::{run_pipeline, PipelineOutcome};
use rsp_host_executor::{
    alerting::AlertingClient, create_eth_block_execution_strategy_factory, EthExecutorComponents,
    FullExecutor,
};
use rsp_provider::create_provider;
use sp1_sdk::{include_elf, ProverClient};
use summary::ProvingSummary;
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod cli;
mod ethproofs;
mod metrics;
mod pipeline;
mod summary;

/// How long to wait at shutdown for in-flight ethproofs submissions to complete, so a
/// just-proved block's submission isn't aborted by the runtime teardown.
const SUBMISSION_DRAIN_TIMEOUT: Duration = Duration::from_secs(60);

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Initialize the environment variables.
    dotenv::dotenv().ok();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    // Initialize the logger.
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::from_default_env()
                .add_directive("sp1_core_machine=warn".parse().unwrap())
                .add_directive("sp1_core_executor=warn".parse().unwrap())
                .add_directive("sp1_prover=warn".parse().unwrap()),
        )
        .init();

    // Parse the command line arguments.
    let args = Args::parse();
    let config = args.as_config().await?;

    // Install the Prometheus exporter when an address is configured.
    if let Some(metrics_addr) = args.metrics_addr()? {
        install_prometheus_exporter(metrics_addr)?;
    }

    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, None);

    // Report to ethproofs and collect internal metrics, side by side. ethproofs submission is
    // disabled unless both the endpoint and API token are configured, so the service can run
    // (execute, prove, collect metrics) locally without credentials.
    let ethproofs_client = EthproofsClient::new(
        args.ethproofs_cluster_id,
        args.ethproofs_endpoint(),
        args.ethproofs_api_token(),
    );
    if !ethproofs_client.is_enabled() {
        warn!("ethproofs submission disabled (endpoint and/or API token not set); running locally");
    }
    // Keep a handle so in-flight submissions can be drained before exiting.
    let submissions = ethproofs_client.clone();
    // Accumulates timing for an end-of-run summary; a handle is kept for the summary log after
    // the pipeline stops. Composed via nested tuples (the fan-out impl is for pairs).
    let summary = ProvingSummary::new();
    let hooks = (ethproofs_client, (MetricsHook::new(), summary.clone()));
    let alerting_client = args.pager_duty_integration_key().map(AlertingClient::new).map(Arc::new);

    let ws = WsConnect::new(args.ws_rpc_url);
    let ws_provider = ProviderBuilder::new().connect_ws(ws).await?;
    let http_provider = create_provider(args.http_rpc_url);

    let prover_client = ProverClient::from_env().await;
    let client = Arc::new(prover_client);

    let executor = FullExecutor::<EthExecutorComponents<_, _>, _>::try_new(
        http_provider.clone(),
        elf,
        block_execution_strategy_factory,
        client,
        hooks,
        config,
    )
    .await?;

    info!("Latest block number: {}", http_provider.get_block_number().await?);

    // Subscribe to block headers and drain the subscription eagerly on a dedicated task, so a
    // backlogged pipeline can never park the WS broadcast buffer into overflowing and silently
    // dropping headers. Sampled block numbers are buffered without bound instead: they are just
    // integers, and accumulating a backlog (rather than dropping blocks) is the desired behavior
    // when proving falls behind — the rsp_ethproofs_chain_head_block gauge (vs last_proved_block)
    // exposes the lag so operators can alert on it.
    let block_interval = args.block_interval;
    let mut subscription = ws_provider.subscribe_blocks().await?;
    let (block_tx, block_rx) = mpsc::unbounded_channel();
    let drain_alerting = alerting_client.clone();

    tokio::spawn(async move {
        loop {
            match subscription.recv().await {
                Ok(header) => {
                    metrics::record_chain_head(header.number);

                    if header.number.is_multiple_of(block_interval) {
                        metrics::record_block_sampled();
                        if block_tx.send(header.number).is_err() {
                            break;
                        }
                    }
                }
                // This task always polls promptly, so lag means the runtime itself is starved;
                // make any dropped headers loud instead of silent.
                Err(RecvError::Lagged(skipped)) => {
                    warn!("WS block subscription lagged; {skipped} headers were dropped");
                }
                // Terminal: alert before breaking, since dropping the sender ends the pipeline
                // and the process.
                Err(RecvError::Closed) => {
                    error!("WS block subscription closed; no further blocks will be received");
                    if let Some(alerting) = &drain_alerting {
                        alerting
                            .send_alert(
                                "ethproofs: the WS block subscription closed; the service is \
                                 exiting"
                                    .to_string(),
                            )
                            .await;
                    }
                    break;
                }
            }
        }
    });

    let outcome = run_pipeline(
        executor,
        UnboundedReceiverStream::new(block_rx),
        alerting_client,
        args.max_blocks,
    )
    .await;

    // Give in-flight ethproofs submissions a chance to finish before the runtime tears down.
    submissions.drain_submissions(SUBMISSION_DRAIN_TIMEOUT).await;

    // Log the aggregate timing summary of the run.
    summary.log_summary();

    match outcome {
        // A bounded `--max-blocks` run finished its quota: a clean, intentional stop. Exit with
        // success so exit-code-based supervision does not restart it.
        PipelineOutcome::ReachedBlockLimit => {
            info!("Reached the configured block limit; exiting cleanly");
            Ok(())
        }
        // The pipeline otherwise only ends when the block stream does, i.e. the WS subscription
        // closed for good. Exit with an error so exit-code-based supervision (systemd
        // Restart=on-failure) restarts the service with a fresh connection.
        PipelineOutcome::StreamEnded => {
            bail!(
                "the WS block subscription closed; exiting so the supervisor restarts the service"
            )
        }
    }
}
