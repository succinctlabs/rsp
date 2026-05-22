use std::sync::Arc;

use alloy_provider::{Provider, ProviderBuilder, WsConnect};
use clap::Parser;
use cli::Args;
use eth_proofs::EthProofsClient;
use futures::{future::ready, StreamExt};
use metrics::{install_prometheus_exporter, MetricsHook};
use pipeline::run_pipeline;
use rsp_host_executor::{
    alerting::AlertingClient, create_eth_block_execution_strategy_factory, EthExecutorComponents,
    FullExecutor,
};
use rsp_provider::create_provider;
use sp1_sdk::{include_elf, ProverClient};
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod cli;
mod eth_proofs;
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
    let prove_mode = config.prove_mode;

    // Install the Prometheus exporter when an address is configured.
    if let Some(metrics_addr) = args.metrics_addr {
        install_prometheus_exporter(metrics_addr)?;
    }

    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, None);

    // Report to eth-proofs and collect internal metrics, side by side.
    let hooks = (
        EthProofsClient::new(
            args.eth_proofs_cluster_id,
            args.eth_proofs_endpoint,
            args.eth_proofs_api_token,
        ),
        MetricsHook::new(),
    );
    let alerting_client = args.pager_duty_integration_key.map(AlertingClient::new);

    let ws = WsConnect::new(args.ws_rpc_url);
    let ws_provider = ProviderBuilder::new().connect_ws(ws).await?;
    let http_provider = create_provider(args.http_rpc_url);

    if args.moongate_endpoint.is_some() {
        tracing::warn!("moongate_endpoint is not supported in SP1 v6, using local CudaProver");
    }
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

    // Subscribe to block headers, keeping only the blocks we want to execute.
    let block_interval = args.block_interval;
    let blocks = ws_provider
        .subscribe_blocks()
        .await?
        .into_stream()
        .filter(move |h| ready(h.number.is_multiple_of(block_interval)))
        .map(|h| h.number);

    run_pipeline(executor, blocks, prove_mode, alerting_client).await
}
