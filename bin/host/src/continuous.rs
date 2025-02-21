use crate::{
    cli::{HostArgs, ProviderConfig},
    db,
    execute::execute,
};

use alloy_provider::{Provider, ProviderBuilder};
use futures_util::{future::join_all, StreamExt};
use op_alloy_network::Ethereum;
use rsp_host_executor::create_eth_block_execution_strategy_factory;
use rsp_primitives::genesis::Genesis;
use std::{pin::Pin, time::SystemTime};
use tokio::task;

pub async fn run_continuous_mode(
    args: HostArgs,
    provider_config: ProviderConfig,
    genesis: Genesis,
) -> eyre::Result<()> {
    println!("🔁 running rsp host in continuous mode");
    let provider_args = args.provider.clone();
    let max_threads = args.num_threads;

    // change https to wss
    let ws_url =
        provider_args.into_provider().await?.rpc_url.unwrap().to_string().replace("https", "wss");

    let ws = alloy::providers::WsConnect::new(ws_url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;
    let subscription = provider.subscribe_blocks().await?;
    let mut stream = subscription.into_stream().take(3);

    let mut handles: Vec<task::JoinHandle<()>> = Vec::new();

    while let Some(block) = stream.next().await {
        let block_num = block.number;
        println!("Received block: {:?}", block_num);

        // Clone the args and modify the block number with the one received from the stream
        let mut modified_args = args.clone();
        modified_args.block_number = Some(block_num);

        let provider_config_clone = provider_config.clone();
        let genesis_clone = genesis.clone();

        // Spawn a new task to process the block
        let handle = task::spawn(async move {
            if let Err(e) =
                spawn_block_processing_task(modified_args, provider_config_clone, genesis_clone)
                    .await
            {
                eprintln!("Block processing failed: {}", e);
            }
        });

        handles.push(handle);

        if handles.len() >= max_threads {
            let (completed, _index, pending) = futures_util::future::select_all(handles).await;
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

    Ok(())
}

pub async fn spawn_block_processing_task(
    args: HostArgs,
    provider_config: ProviderConfig,
    genesis: Genesis,
) -> eyre::Result<()> {
    let block_num = args.block_number.unwrap();
    println!("Processing block {:?}", &block_num);

    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&genesis, args.custom_beneficiary);

    match execute::<Ethereum, _, _>(
        args.clone(),
        provider_config.clone(),
        genesis.clone(),
        None,
        block_execution_strategy_factory,
        false,
    )
    .await
    {
        Ok(_) => {
            println!("Successfully processed block {:?}", &block_num);
            Ok(())
        }
        Err(e) => {
            if let Err(db_err) = update_block_status_as_failed(&args.db_url, block_num).await {
                eprintln!("Database error updating block {:?}: {}", &block_num, db_err);
            }
            eprintln!("Error processing block {:?}: {}", &block_num, e);

            // Recursive retry
            //
            // [IMPORTANT] this recursive retry is not ideal, but it's a quick fix to handle failures
            // it is assumed that the error is transient and will be resolved in the next attempt
            println!("Retrying block {:?}...", &block_num);
            Pin::from(Box::new(spawn_block_processing_task(
                args.clone(),
                provider_config.clone(),
                genesis.clone(),
            )))
            .await
        }
    }
}

async fn update_block_status_as_failed(db_url: &str, block_num: u64) -> eyre::Result<()> {
    let pool = db::init_db_pool(db_url).await?;
    let end_time = db::system_time_to_timestamp(SystemTime::now());
    db::update_block_status_as_failed(&pool, block_num as i64, end_time).await?;
    Ok(())
}
