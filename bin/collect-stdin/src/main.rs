#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use std::{path::PathBuf, sync::Arc};

use alloy_chains::Chain;
use alloy_provider::{network::AnyNetwork, Provider, RootProvider};
use clap::Parser;
use rsp_host_executor::{
    create_eth_block_execution_strategy_factory, Config, EthExecutorComponents, HostExecutor,
};
use rsp_primitives::genesis::Genesis;
use rsp_provider::create_provider;
use sp1_sdk::{include_elf, SP1Stdin};
use tracing_subscriber::{
    filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};
use url::Url;

#[derive(Debug, Clone, Parser)]
pub struct Args {
    /// The block numbers to collect (comma-separated)
    #[clap(long, value_delimiter = ',')]
    pub blocks: Vec<u64>,

    /// The rpc url used to fetch data about the block
    #[clap(long, env = "RPC_1")]
    pub rpc_url: Url,

    /// Output directory for the stdin files
    #[clap(long)]
    pub output_dir: PathBuf,

    /// Whether to also copy the ELF to the output directory
    #[clap(long)]
    pub copy_elf: bool,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    dotenv::dotenv().ok();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    tracing_subscriber::registry().with(fmt::layer()).with(EnvFilter::from_default_env()).init();

    let args = Args::parse();

    // Create output directory
    std::fs::create_dir_all(&args.output_dir)?;

    // Get chain ID from RPC
    let provider = RootProvider::<AnyNetwork>::new_http(args.rpc_url.clone());
    let chain_id = provider.get_chain_id().await?;
    let chain = Chain::from_id(chain_id);

    tracing::info!("Connected to chain {}", chain_id);

    let genesis: Genesis = chain_id.try_into()?;
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&genesis, None);

    let config = Config {
        chain,
        genesis: genesis.clone(),
        rpc_url: Some(args.rpc_url.clone()),
        cache_dir: None,
        custom_beneficiary: None,
        prove_mode: None,
        skip_client_execution: true,
        opcode_tracking: false,
    };

    let host_executor = HostExecutor::new(
        block_execution_strategy_factory,
        Arc::new(
            <EthExecutorComponents<()> as rsp_host_executor::ExecutorComponents>::try_into_chain_spec(&config.genesis)?,
        ),
    );

    let rpc_provider = create_provider(args.rpc_url);

    // Get the ELF
    let elf = include_elf!("rsp-client").to_vec();

    // Copy ELF if requested
    if args.copy_elf {
        let elf_path = args.output_dir.join("rsp-client.elf");
        std::fs::write(&elf_path, &elf)?;
        tracing::info!("Saved ELF to {:?}", elf_path);
    }

    // Collect stdin for each block
    for block_number in &args.blocks {
        tracing::info!("Collecting stdin for block {}", block_number);

        let client_input = host_executor
            .execute(
                *block_number,
                &rpc_provider,
                config.genesis.clone(),
                config.custom_beneficiary,
                config.opcode_tracking,
            )
            .await?;

        // Create SP1Stdin exactly as the executor does
        let mut stdin = SP1Stdin::new();
        let buffer = bincode::serialize(&client_input)?;
        stdin.write_vec(buffer);

        // Serialize the SP1Stdin
        let stdin_bytes = bincode::serialize(&stdin)?;

        // Save to file
        let output_path = args.output_dir.join(format!("{}.bin", block_number));
        std::fs::write(&output_path, &stdin_bytes)?;

        tracing::info!(
            "Saved SP1Stdin for block {} to {:?} ({} bytes)",
            block_number,
            output_path,
            stdin_bytes.len()
        );
    }

    tracing::info!("Done collecting {} blocks", args.blocks.len());

    Ok(())
}
