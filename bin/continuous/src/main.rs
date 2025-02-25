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
    create_eth_block_execution_strategy_factory, BlockExecutor, Config, FullExecutor,
};
use sp1_sdk::include_elf;
use sqlx::migrate::Migrator;
use tokio::{sync::Semaphore, task};
use tracing::{error, info};
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
    let max_retries = args.execution_retries.unwrap_or(0);

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
    let mut stream = subscription.into_stream();

    let concurrent_executions_semaphore = Arc::new(Semaphore::new(args.max_concurrent_executions));

    while let Some(header) = stream.next().await {
        info!("Received block: {:?}", header.number);

        let executor = executor.clone();
        let db_pool = db_pool.clone();
        let concurrent_executions_semaphore = concurrent_executions_semaphore.clone();

        task::spawn(async move {
            let permit = concurrent_executions_semaphore.try_acquire();

            if permit.is_err() {
                error!("Maximum concurrent executions reached: Skipping block {}", header.number);
                return;
            }

            match process_block(header.number, executor, max_retries).await {
                Ok(_) => info!("Successfully processed block {}", header.number),
                Err(err) => {
                    error!("Error executing block {}: {}", header.number, err);

                    if let Err(err) = db::update_block_status_as_failed(
                        &db_pool,
                        header.number,
                        SystemTime::now(),
                    )
                    .await
                    {
                        error!(
                            "Database error whileupdating block status {}: {}",
                            header.number, err
                        );
                    }
                }
            }
        });
    }

    Ok(())
}

async fn process_block<P, F>(
    block_number: u64,
    executor: Arc<FullExecutor<P, Ethereum, EthPrimitives, F, PersistToPostgres>>,
    max_retries: usize,
) -> eyre::Result<()>
where
    P: Provider<Ethereum> + Clone,
    F: BlockExecutionStrategyFactory<Primitives = EthPrimitives>,
{
    let mut retry_count = 0;

    loop {
        match executor.execute(block_number).await {
            Ok(_) => {
                return Ok(());
            }
            Err(e) => {
                retry_count += 1;
                if retry_count > max_retries {
                    error!("Max retries reached for block: {block_number}");
                    return Err(e);
                }
            }
        }
    }
}
