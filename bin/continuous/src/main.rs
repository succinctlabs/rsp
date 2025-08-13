use std::sync::Arc;

use alloy_provider::{network::Ethereum, Provider, ProviderBuilder, WsConnect};
use clap::Parser;
use cli::Args;
use db::PersistToPostgres;
use futures_util::StreamExt;
use rsp_host_executor::{
    alerting::AlertingClient, create_eth_block_execution_strategy_factory, BlockExecutor, Config,
    EthExecutorComponents, ExecutorComponents, FullExecutor,
};
use rsp_provider::create_provider;
use sp1_sdk::{include_elf, EnvProver};
use tokio::{sync::Semaphore, task};
use tracing::{error, info, instrument, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod db;

mod cli;

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

    let args = Args::parse();
    let config = Config::mainnet();

    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, None);

    let db_pool = db::build_db_pool(&args.database_url).await?;
    let ws = WsConnect::new(args.ws_rpc_url);
    let ws_provider = ProviderBuilder::new().connect_ws(ws).await?;
    let http_provider = create_provider(args.http_rpc_url);
    let alerting_client =
        args.pager_duty_integration_key.map(|key| Arc::new(AlertingClient::new(key)));
    let prover_client = Arc::new(EnvProver::new());

    let executor = Arc::new(
        FullExecutor::<EthExecutorComponents<_>, _>::try_new(
            http_provider.clone(),
            elf,
            block_execution_strategy_factory,
            prover_client,
            PersistToPostgres::new(db_pool.clone()),
            config,
        )
        .await?,
    );

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
                    let error_message = format!("Error executing block {block_number}: {err}");
                    error!("{error_message}");

                    if let Some(alerting_client) = &alerting_client {
                        alerting_client
                            .send_alert(format!("OP Succinct Explorer (RSP) - {error_message}"))
                            .await;
                    }

                    if let Err(err) =
                        db::update_block_status_as_failed(&db_pool, block_number).await
                    {
                        let error_message = format!(
                            "Database error while updating block {block_number} status: {err}",
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
async fn process_block<C, P>(
    number: u64,
    executor: Arc<FullExecutor<C, P>>,
    max_retries: usize,
) -> eyre::Result<()>
where
    C: ExecutorComponents<Network = Ethereum>,
    P: Provider<Ethereum> + Clone + std::fmt::Debug,
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
