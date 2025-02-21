#![warn(unused_crate_dependencies)]

use clap::Parser;
use std::fs;
use tracing_subscriber::{
    filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

use rsp_primitives::genesis::Genesis;

mod block;
mod cli;
mod continuous;
mod db;
mod eth_proofs;
mod execute;

use block::process_single_block;
use cli::{HostArgs, ProviderConfig};
use continuous::run_continuous_mode;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    initialize_environment()?;
    let (args, provider_config, genesis) = setup_execution_context().await?;

    if args.block_number.is_none() {
        run_continuous_mode(args, provider_config, genesis).await?;
    } else {
        process_single_block(args, provider_config, genesis).await?;
    }

    Ok(())
}

fn initialize_environment() -> eyre::Result<()> {
    dotenv::dotenv().ok();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    tracing_subscriber::registry().with(fmt::layer()).with(EnvFilter::from_default_env()).init();

    Ok(())
}

async fn setup_execution_context() -> eyre::Result<(HostArgs, ProviderConfig, Genesis)> {
    let args = HostArgs::parse();
    let provider_config = args.provider.clone().into_provider().await?;

    let genesis = if let Some(genesis_path) = &args.genesis_path {
        let genesis_json = fs::read_to_string(genesis_path)
            .map_err(|err| eyre::eyre!("Failed to read genesis file: {err}"))?;
        Genesis::Custom(genesis_json)
    } else {
        provider_config.chain_id.try_into()?
    };

    Ok((args, provider_config, genesis))
}
