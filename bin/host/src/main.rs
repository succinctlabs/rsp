#![warn(unused_crate_dependencies)]

use alloy::signers::k256::elliptic_curve::rand_core::block;
use alloy_chains::Chain;
use alloy_consensus::BlockHeader;
use alloy_primitives::Address;
use alloy_provider::{Network, Provider, ProviderBuilder, RootProvider};
use clap::Parser;
use eth_proofs::EthProofsClient;
use execute::process_execution_report;
use futures_util::future::join_all;
use futures_util::StreamExt;
use mongodb::{
    bson::{doc, Bson, Document},
    options::ClientOptions,
    Client, Collection,
};
use op_alloy_network::{Ethereum, Optimism};
use reth_evm::execute::BlockExecutionStrategyFactory;
use reth_primitives::NodePrimitives;
use reth_primitives_traits::BlockBody;
use rsp_client_executor::{io::ClientExecutorInput, IntoInput, IntoPrimitives};
use rsp_host_executor::{
    create_eth_block_execution_strategy_factory, create_op_block_execution_strategy_factory,
    HostExecutor,
};
use rsp_primitives::genesis::Genesis;
use rsp_rpc_db::RpcDb;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sp1_sdk::{include_elf, network::B256, ProverClient, SP1Stdin};
use sqlx::{postgres::PgPoolOptions, Pool, Postgres};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, path::PathBuf};

use tokio::task;
use tracing_subscriber::{
    filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

mod execute;

mod cli;
use cli::{ProviderArgs, ProviderConfig};

mod eth_proofs;

/// The arguments for the host executable.
#[derive(Debug, Clone, Parser)]
struct HostArgs {
    /// The block number of the block to execute.
    #[clap(long)]
    block_number: Option<u64>,

    #[clap(flatten)]
    provider: ProviderArgs,

    // database connection
    #[clap(long)]
    db_url: String,

    /// The path to the genesis json file to use for the execution.
    #[clap(long)]
    genesis_path: Option<PathBuf>,

    /// The custom beneficiary address, used with Clique consensus.
    #[clap(long)]
    custom_beneficiary: Option<Address>,

    /// Whether to generate a proof or just execute the block.
    #[clap(long)]
    prove: bool,

    /// Optional path to the directory containing cached client input. A new cache file will be
    /// created from RPC data if it doesn't already exist.
    #[clap(long)]
    cache_dir: Option<PathBuf>,

    /// The path to the CSV file containing the execution data.
    #[clap(long, default_value = "report.csv")]
    report_path: PathBuf,

    /// Optional ETH proofs endpoint.
    #[clap(long, env, requires("eth_proofs_api_token"))]
    eth_proofs_endpoint: Option<String>,

    /// Optional ETH proofs API token.
    #[clap(long, env)]
    eth_proofs_api_token: Option<String>,

    /// Optional ETH proofs cluster ID.
    #[clap(long, default_value_t = 1)]
    eth_proofs_cluster_id: u64,
}

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
                    Err(e) => eprintln!("Error processing block {}: {}", block_num, e),
                }

                println!("Processed block {}", block_num);
            });

            handles.push(handle);

            if handles.len() >= 8 {
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

#[derive(Debug, Serialize, Deserialize)]
struct ProvableBlock {
    block_number: i64,
    status: String,
    gas_used: i64,
    tx_count: i64,
    num_cycles: i64,
    start_time: Option<i64>,
    end_time: Option<i64>,
}

async fn init_db_pool(db_url: &str) -> Result<Pool<Postgres>, sqlx::Error> {
    let database_url = db_url;
    PgPoolOptions::new().max_connections(8).connect(database_url).await
}

async fn init_db_schema(pool: &Pool<Postgres>) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS rsp_blocks (
            block_number BIGINT PRIMARY KEY,
            status VARCHAR(50) NOT NULL,
            gas_used BIGINT NOT NULL,
            tx_count BIGINT NOT NULL,
            num_cycles BIGINT NOT NULL,
            start_time BIGINT,
            end_time BIGINT
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

async fn insert_block(pool: &Pool<Postgres>, block: &ProvableBlock) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO rsp_blocks 
        (block_number, status, gas_used, tx_count, num_cycles, start_time, end_time)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
    )
    .bind(block.block_number)
    .bind(&block.status)
    .bind(block.gas_used)
    .bind(block.tx_count)
    .bind(block.num_cycles)
    .bind(block.start_time)
    .bind(block.end_time)
    .execute(pool)
    .await?;

    Ok(())
}

async fn update_block_status(
    pool: &Pool<Postgres>,
    block_number: i64,
    gas_used: i64,
    tx_count: i64,
    num_cycles: i64,
    end_time: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE rsp_blocks
        SET status = 'executed',
            gas_used = $2,
            tx_count = $3,
            num_cycles = $4,
            end_time = $5
        WHERE block_number = $1
        "#,
    )
    .bind(block_number)
    .bind(gas_used)
    .bind(tx_count)
    .bind(num_cycles)
    .bind(end_time)
    .execute(pool)
    .await?;

    Ok(())
}

fn system_time_to_timestamp(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

async fn execute<N, NP, F>(
    args: HostArgs,
    provider_config: ProviderConfig,
    genesis: Genesis,
    eth_proofs_client: Option<EthProofsClient>,
    block_execution_strategy_factory: F,
    is_optimism: bool,
) -> eyre::Result<()>
where
    N: Network,
    NP: NodePrimitives + DeserializeOwned,
    F: BlockExecutionStrategyFactory<Primitives = NP>,
    F::Primitives: IntoPrimitives<N> + IntoInput,
{
    // Initialize PostgreSQL connection pool
    let pool = init_db_pool(&args.db_url).await?;

    // Initialize database schema
    init_db_schema(&pool).await?;

    let start_time = system_time_to_timestamp(SystemTime::now());

    // Create new block record
    let block = ProvableBlock {
        block_number: args.block_number.unwrap() as i64,
        status: "queued".to_string(),
        gas_used: 0,
        tx_count: 0,
        num_cycles: 0,
        start_time: Some(start_time),
        end_time: None,
    };
    insert_block(&pool, &block).await?;

    let client_input_from_cache = try_load_input_from_cache::<NP>(
        args.cache_dir.as_ref(),
        provider_config.chain_id,
        args.block_number.unwrap(),
    )?;

    let client_input = match (client_input_from_cache, provider_config.rpc_url) {
        (Some(client_input_from_cache), _) => client_input_from_cache,
        (None, Some(rpc_url)) => {
            let provider = RootProvider::<N>::new_http(rpc_url);

            // Setup the host executor.
            let host_executor = HostExecutor::new(block_execution_strategy_factory);

            let rpc_db = RpcDb::new(provider.clone(), args.block_number.unwrap() - 1);

            // Execute the host.
            let client_input = host_executor
                .execute(
                    args.block_number.unwrap(),
                    &rpc_db,
                    &provider,
                    genesis,
                    args.custom_beneficiary,
                )
                .await?;

            if let Some(ref cache_dir) = args.cache_dir {
                let input_folder = cache_dir.join(format!("input/{}", provider_config.chain_id));
                if !input_folder.exists() {
                    std::fs::create_dir_all(&input_folder)?;
                }

                let input_path = input_folder.join(format!("{}.bin", args.block_number.unwrap()));
                let mut cache_file = std::fs::File::create(input_path)?;

                bincode::serialize_into(&mut cache_file, &client_input)?;
            }

            client_input
        }
        (None, None) => {
            eyre::bail!("cache not found and RPC URL not provided")
        }
    };

    // Generate the proof.
    let client = ProverClient::from_env();

    // Setup the proving key and verification key.
    let (pk, vk) = client.setup(if is_optimism {
        include_elf!("rsp-client-op")
    } else {
        include_elf!("rsp-client")
    });

    // Execute the block inside the zkVM.
    let mut stdin = SP1Stdin::new();
    let buffer = bincode::serialize(&client_input).unwrap();

    stdin.write_vec(buffer);

    // Only execute the program.
    let (mut public_values, execution_report) = client.execute(&pk.elf, &stdin).run().unwrap();

    // Read the block hash.
    let block_hash = public_values.read::<B256>();
    println!("success: block_hash={block_hash}");

    let executed_block = client_input.clone().current_block;

    if eth_proofs_client.is_none() {
        // Process the execute report, print it out, and save data to a CSV specified by
        // report_path.
        process_execution_report(
            provider_config.chain_id,
            client_input,
            &execution_report,
            args.report_path.clone(),
        )?;
    }

    let end_time = system_time_to_timestamp(SystemTime::now());

    // Update the block status in PostgreSQL
    update_block_status(
        &pool,
        args.block_number.unwrap() as i64,
        executed_block.header.gas_used() as i64,
        executed_block.body.transaction_count() as i64,
        execution_report.total_instruction_count() as i64,
        end_time,
    )
    .await?;

    if args.prove {
        println!("Starting proof generation.");

        if let Some(eth_proofs_client) = &eth_proofs_client {
            eth_proofs_client.proving(args.block_number.unwrap()).await?;
        }

        let start = std::time::Instant::now();
        let proof = client.prove(&pk, &stdin).compressed().run().expect("Proving should work.");
        let proof_bytes = bincode::serialize(&proof.proof).unwrap();
        let elapsed = start.elapsed().as_secs_f32();

        if let Some(eth_proofs_client) = &eth_proofs_client {
            eth_proofs_client
                .proved(&proof_bytes, args.block_number.unwrap(), &execution_report, elapsed, &vk)
                .await?;
        }
    }

    Ok(())
}

fn try_load_input_from_cache<P: NodePrimitives + DeserializeOwned>(
    cache_dir: Option<&PathBuf>,
    chain_id: u64,
    block_number: u64,
) -> eyre::Result<Option<ClientExecutorInput<P>>> {
    Ok(if let Some(cache_dir) = cache_dir {
        let cache_path = cache_dir.join(format!("input/{}/{}.bin", chain_id, block_number));

        if cache_path.exists() {
            // TODO: prune the cache if invalid instead
            let mut cache_file = std::fs::File::open(cache_path)?;
            let client_input: ClientExecutorInput<P> = bincode::deserialize_from(&mut cache_file)?;

            Some(client_input)
        } else {
            None
        }
    } else {
        None
    })
}
