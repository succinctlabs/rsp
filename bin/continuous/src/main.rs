use std::{sync::Arc, time::SystemTime};

use alloy_provider::{network::Ethereum, Provider, ProviderBuilder, WsConnect};
use alloy_rpc_client::RpcClient;
use alloy_transport::layers::RetryBackoffLayer;
use clap::Parser;
use cli::Args;
use db::PersistToPostgres;
use futures_util::StreamExt;
use reth_evm::execute::BlockExecutionStrategyFactory;
use reth_primitives::EthPrimitives;
use rsp_host_executor::{
    alerting::AlertingClient, create_eth_block_execution_strategy_factory, BlockExecutor, Config,
    FullExecutor,
};
use sp1_sdk::include_elf;
use sqlx::migrate::Migrator;
use tokio::{sync::Semaphore, task};
use tracing::{error, info, instrument, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod db;

mod cli;

static MIGRATOR: Migrator = sqlx::migrate!();

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Initialize the environment variables.
    dotenv::dotenv().ok();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    // Initialize the logger.
    tracing_subscriber::registry().with(fmt::layer()).with(EnvFilter::from_default_env()).init();

    let args = Args::parse();
    let config = Config::mainnet();

    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, None);

    let db_pool = db::build_db_pool(&args.db_url).await?;
    let ws = WsConnect::new(args.ws_rpc_url);
    let ws_provider = ProviderBuilder::new().on_ws(ws).await?;
    let retry_layer = RetryBackoffLayer::new(3, 1000, 100);
    let client = RpcClient::builder().layer(retry_layer).http(args.http_rpc_url);
    let http_provider = ProviderBuilder::new().network::<Ethereum>().on_client(client);
    let alerting_client =
        args.pager_duty_integration_key.map(|key| Arc::new(AlertingClient::new(key)));

    // Create or update the database schema.
    MIGRATOR.run(&db_pool).await?;

    let executor = Arc::new(FullExecutor::new(
        http_provider.clone(),
        elf,
        block_execution_strategy_factory,
        PersistToPostgres::new(db_pool.clone()),
        config,
    ));

    // Subscribe to block headers.
    let subscription = ws_provider.subscribe_blocks().await?;
    let mut stream = subscription.into_stream().map(|h| h.number);

    let concurrent_executions_semaphore = Arc::new(Semaphore::new(args.max_concurrent_executions));

    while let Some(block_number) = stream.next().await {
        info!("Received block: {:?}", block_number);

        let executor = executor.clone();
        let db_pool = db_pool.clone();
        let alerting_client = alerting_client.clone();
        let permit = concurrent_executions_semaphore.clone().acquire_owned().await?;

        task::spawn(async move {
            match process_block(block_number, executor, args.execution_retries).await {
                Ok(_) => info!("Successfully processed block {}", block_number),
                Err(err) => {
                    let error_message = format!("Error executing block {}: {}", block_number, err);
                    error!("{error_message}");

                    if let Some(alerting_client) = &alerting_client {
                        alerting_client
                            .send_alert(format!("OP Succinct Explorer (RSP) - {error_message}"))
                            .await;
                    }

                    if let Err(err) =
                        db::update_block_status_as_failed(&db_pool, block_number, SystemTime::now())
                            .await
                    {
                        let error_message = format!(
                            "Database error while updating block {} status: {}",
                            block_number, err
                        );

                        error!("{error_message}",);

                        if let Some(alerting_client) = &alerting_client {
                            alerting_client
                                .send_alert(format!("OP Succinct Explorer (RSP) - {error_message}"))
                                .await;
                        }
                    }
                }
            }

            drop(permit);
        });
    }

    Ok(())
}

#[instrument(skip(executor, max_retries))]
async fn process_block<P, F>(
    number: u64,
    executor: Arc<FullExecutor<P, Ethereum, EthPrimitives, F, PersistToPostgres>>,
    max_retries: usize,
) -> eyre::Result<()>
where
    P: Provider<Ethereum> + Clone,
    F: BlockExecutionStrategyFactory<Primitives = EthPrimitives>,
{
    let mut retry_count = 0;

    // Wait for the block to be avaliable in the HTTP provider
    executor.wait_for_block(number).await?;

    loop {
        match executor.execute(number).await {
            Ok(_) => {
                return Ok(());
            }
            Err(err) => {
                warn!("Failed to execute block {number}: {err}, retrying...");
                retry_count += 1;
                if retry_count > max_retries {
                    error!("Max retries reached for block: {number}");
                    return Err(err);
                }
            }
        }
    }
}
