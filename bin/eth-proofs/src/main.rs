use alloy_provider::{network::Ethereum, Provider, ProviderBuilder, WsConnect};
use alloy_rpc_client::RpcClient;
use alloy_transport::layers::RetryBackoffLayer;
use clap::Parser;
use cli::Args;
use eth_proofs::EthProofsClient;
use futures::{future::ready, StreamExt};
use pagerduty_rs::{
    eventsv2async::EventsV2,
    types::{AlertTrigger, AlertTriggerPayload, Event, Severity},
};
use rsp_host_executor::{create_eth_block_execution_strategy_factory, BlockExecutor, FullExecutor};
use sp1_sdk::include_elf;
use time::OffsetDateTime;
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
    tracing_subscriber::registry().with(fmt::layer()).with(EnvFilter::from_default_env()).init();

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

    let ev2 = args.pager_duty_integration_key.map(|key| EventsV2::new(key, None)).transpose()?;
    let ws = WsConnect::new(args.ws_rpc_url);
    let ws_provider = ProviderBuilder::new().on_ws(ws).await?;
    let retry_layer = RetryBackoffLayer::new(3, 1000, 100);
    let client = RpcClient::builder().layer(retry_layer).http(args.http_rpc_url);
    let http_provider = ProviderBuilder::new().network::<Ethereum>().on_client(client);

    // Subscribe to block headers.
    let subscription = ws_provider.subscribe_blocks().await?;
    let mut stream =
        subscription.into_stream().filter(|h| ready(h.number % args.block_interval == 0));

    let mut executor = FullExecutor::new(
        http_provider.clone(),
        elf,
        block_execution_strategy_factory,
        eth_proofs_client,
        config,
    );

    info!("Latest block number: {}", http_provider.get_block_number().await?);

    while let Some(header) = stream.next().await {
        if let Err(err) = executor.execute(header.number).await {
            let error_message = format!("Error handling block {}: {err}", header.number);
            error!(error_message);

            if let Some(ref ev2) = ev2 {
                send_alert(ev2, error_message, Severity::Error).await;
            }
        }
    }

    if let Some(ref ev2) = ev2 {
        send_alert(ev2, "Eth proofs exited".to_string(), Severity::Critical).await;
    }

    Ok(())
}

async fn send_alert(ev2: &EventsV2, summary: String, severity: Severity) {
    if let Err(err) = ev2
        .event(Event::AlertTrigger(AlertTrigger {
            payload: AlertTriggerPayload::<()> {
                severity,
                summary,
                source: Default::default(),
                timestamp: Some(OffsetDateTime::now_utc()),
                component: Some("eth-proofs".to_string()),
                group: None,
                class: None,
                custom_details: None,
            },
            dedup_key: None,
            images: None,
            links: None,
            client: None,
            client_url: None,
        }))
        .await
    {
        error!("Error sending alert: {err}");
    }
}
