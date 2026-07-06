use std::sync::Arc;

use alloy_provider::{Provider, ProviderBuilder, WsConnect};
use clap::Parser;
use cli::Args;
use ethproofs::EthproofsClient;
use metrics::{install_prometheus_exporter, MetricsHook};
use pipeline::run_pipeline;
use rsp_host_executor::{
    alerting::AlertingClient, create_eth_block_execution_strategy_factory, EthExecutorComponents,
    FullExecutor,
};
use rsp_provider::create_provider;
use sp1_sdk::{include_elf, ProverClient};
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod cli;
mod ethproofs;
mod metrics;
mod pipeline;

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
        args.ethproofs_endpoint.filter(|s| !s.is_empty()),
        args.ethproofs_api_token.filter(|s| !s.is_empty()),
    );
    if !ethproofs_client.is_enabled() {
        tracing::warn!(
            "ethproofs submission disabled (endpoint and/or API token not set); running locally"
        );
    }
    let hooks = (ethproofs_client, MetricsHook::new());
    // An empty env var (e.g. from an untouched `.env.example`) means "disabled", like unset.
    let alerting_client =
        args.pager_duty_integration_key.filter(|key| !key.is_empty()).map(AlertingClient::new);

    let ws = WsConnect::new(args.ws_rpc_url);
    let ws_provider = ProviderBuilder::new().connect_ws(ws).await?;
    let http_provider = create_provider(args.http_rpc_url);

    let client = Arc::new(ProverClient::builder().cuda().build().await);

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
    // when proving falls behind.
    let block_interval = args.block_interval;
    let mut subscription = ws_provider.subscribe_blocks().await?;
    let (block_tx, block_rx) = mpsc::unbounded_channel();

    tokio::spawn(async move {
        loop {
            match subscription.recv().await {
                Ok(header) => {
                    if header.number.is_multiple_of(block_interval) &&
                        block_tx.send(header.number).is_err()
                    {
                        break;
                    }
                }
                // This task always polls promptly, so lag means the runtime itself is starved;
                // make any dropped headers loud instead of silent.
                Err(RecvError::Lagged(skipped)) => {
                    warn!("WS block subscription lagged; {skipped} headers were dropped");
                }
                Err(RecvError::Closed) => {
                    error!("WS block subscription closed; no further blocks will be received");
                    break;
                }
            }
        }
    });

    run_pipeline(executor, UnboundedReceiverStream::new(block_rx), alerting_client).await;

    Ok(())
}
