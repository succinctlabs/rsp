use alloy_provider::{network::Ethereum, Provider, ProviderBuilder, WsConnect};
use alloy_rpc_client::RpcClient;
use alloy_transport::layers::RetryBackoffLayer;
use clap::Parser;
use cli::Args;
use eth_proofs::EthProofsClient;
use futures::{future::ready, StreamExt};
use rsp_host_executor::{
    alerting::AlertingClient, create_eth_block_execution_strategy_factory, BlockExecutor,
    FullExecutor,
};
use sp1_sdk::include_elf;
use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod cli;

mod eth_proofs;

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

    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, None);

    let eth_proofs_client = EthProofsClient::new(
        args.eth_proofs_cluster_id,
        args.eth_proofs_endpoint,
        args.eth_proofs_api_token,
    );
    let alerting_client = args.pager_duty_integration_key.map(AlertingClient::new);

    let ws = WsConnect::new(args.ws_rpc_url);
    let ws_provider = ProviderBuilder::new().on_ws(ws).await?;
    let retry_layer = RetryBackoffLayer::new(3, 1000, 100);
    let client = RpcClient::builder().layer(retry_layer).http(args.http_rpc_url);
    let http_provider = ProviderBuilder::new().network::<Ethereum>().on_client(client);

    // Subscribe to block headers.
    let subscription = ws_provider.subscribe_blocks().await?;
    let mut stream =
        subscription.into_stream().filter(|h| ready(h.number % args.block_interval == 0));

    let executor = FullExecutor::try_new(
        http_provider.clone(),
        elf,
        block_execution_strategy_factory,
        eth_proofs_client,
        config,
    )
    .await?;

    info!("Latest block number: {}", http_provider.get_block_number().await?);

    while let Some(header) = stream.next().await {
        // Wait for the block to be avaliable in the HTTP provider
        executor.wait_for_block(header.number).await?;

        if let Err(err) = executor.execute(header.number).await {
            let error_message = format!("Error handling block {}: {err}", header.number);
            error!(error_message);

            if let Some(alerting_client) = &alerting_client {
                alerting_client.send_alert(error_message).await;
            }
        }
    }

    Ok(())
}
