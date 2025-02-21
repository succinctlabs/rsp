#![warn(unused_crate_dependencies)]
use futures_util::{future::join_all, StreamExt};
use std::{fs, time::SystemTime};
use tokio::task;

use alloy_chains::Chain;
use alloy_provider::{Provider, ProviderBuilder};
use clap::Parser;
use tracing_subscriber::{
    filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

use op_alloy_network::{Ethereum, Optimism};
use rsp_host_executor::{
    create_eth_block_execution_strategy_factory, create_op_block_execution_strategy_factory,
};
use rsp_primitives::genesis::Genesis;

mod cli;
mod db;
mod eth_proofs;
mod execute;

use cli::HostArgs;
use eth_proofs::EthProofsClient;
use execute::execute;

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
    let args = HostArgs::parse();
    let provider_config = args.provider.clone().into_provider().await?;

    let genesis = if let Some(genesis_path) = &args.genesis_path {
        let genesis_json = fs::read_to_string(genesis_path)
            .map_err(|err| eyre::eyre!("Failed to read genesis file: {err}"))?;

        Genesis::Custom(genesis_json)
    } else {
        provider_config.chain_id.try_into()?
    };

    let max_threads = args.num_threads;

    if args.block_number.is_none() {
        println!("🔁 running rsp host in continuous mode");
        let provider_args = args.provider.clone();

        // change https to wss
        let ws_url = provider_args
            .into_provider()
            .await?
            .rpc_url
            .unwrap()
            .to_string()
            .replace("https", "wss");

        let ws = alloy::providers::WsConnect::new(ws_url);
        let provider = ProviderBuilder::new().on_ws(ws).await?;
        let subscription = provider.subscribe_blocks().await?;
        let mut stream = subscription.into_stream();

        let block_execution_strategy_factory =
            create_eth_block_execution_strategy_factory(&genesis, args.custom_beneficiary);

        let mut handles: Vec<task::JoinHandle<()>> = Vec::new();

        while let Some(block) = stream.next().await {
            let block_num = block.number;
            println!("Received block: {:?}", block_num);

            let args_clone = args.clone();
            let provider_config_clone = provider_config.clone();
            let genesis_clone = genesis.clone();
            let block_execution_strategy_factory_clone = block_execution_strategy_factory.clone();

            // Spawn a new task for this block
            let handle = task::spawn(async move {
                let mut new_args = args_clone;
                let another_clone = new_args.clone();
                new_args.block_number = Some(block_num);

                println!("Processing block {}", block_num);

                let result = execute::<Ethereum, _, _>(
                    new_args,
                    provider_config_clone,
                    genesis_clone,
                    None,
                    block_execution_strategy_factory_clone,
                    false,
                )
                .await;

                match result {
                    Ok(_) => println!("Successfully processed block {}", block_num),
                    Err(e) => {
                        // update block status as failed
                        let pool = db::init_db_pool(&another_clone.db_url).await.unwrap();
                        let end_time = db::system_time_to_timestamp(SystemTime::now());
                        db::update_block_status_as_failed(&pool, block_num as i64, end_time)
                            .await
                            .unwrap();

                        eprintln!("Error processing block {}: {}", block_num, e)
                    }
                }

                println!("Processed block {}", block_num);
            });

            handles.push(handle);

            if handles.len() >= max_threads {
                let (completed, index, pending) = futures_util::future::select_all(handles).await;
                if let Err(e) = completed {
                    eprintln!("Task error: {}", e);
                }
                handles = pending;
            }
        }

        // Wait for all remaining tasks to complete
        let results = join_all(handles).await;
        for result in results {
            if let Err(e) = result {
                eprintln!("Task error: {}", e);
            }
        }
    } else {
        let block_number = args.block_number.unwrap();

        let is_optimism = Chain::from_id(provider_config.chain_id).is_optimism();

        let eth_proofs_client = EthProofsClient::new(
            args.eth_proofs_cluster_id,
            args.eth_proofs_endpoint.clone(),
            args.eth_proofs_api_token.clone(),
        );

        if let Some(eth_proofs_client) = &eth_proofs_client {
            eth_proofs_client.queued(block_number).await?;
        }

        if is_optimism {
            let block_execution_strategy_factory =
                create_op_block_execution_strategy_factory(&genesis);

            execute::<Optimism, _, _>(
                args,
                provider_config,
                genesis,
                eth_proofs_client,
                block_execution_strategy_factory,
                true,
            )
            .await?;
        } else {
            let block_execution_strategy_factory =
                create_eth_block_execution_strategy_factory(&genesis, args.custom_beneficiary);

            execute::<Ethereum, _, _>(
                args,
                provider_config,
                genesis,
                eth_proofs_client,
                block_execution_strategy_factory,
                false,
            )
            .await?;
        }
    }

    Ok(())
}
