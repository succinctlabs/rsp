#![warn(unused_crate_dependencies)]

use alloy_network::Ethereum;
use alloy_provider::RootProvider;
use alloy_rpc_client::RpcClient;
use alloy_transport::layers::RetryBackoffLayer;
use clap::Parser;
use execute::PersistExecutionReport;
use op_alloy_network::Optimism;
use rsp_host_executor::{
    build_executor, create_eth_block_execution_strategy_factory,
    create_op_block_execution_strategy_factory, BlockExecutor,
};
use sp1_sdk::include_elf;
use tracing_subscriber::{
    filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

mod execute;

mod cli;
use cli::HostArgs;

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
    let block_number = args.block_number;
    let report_path = args.report_path.clone();
    let config = args.as_config().await?;
    let persist_execution_report =
        PersistExecutionReport::new(config.chain.id(), report_path, args.opcode_tracking);
    let rpc_client = config.rpc_url.clone().map(|rpc_url| {
        RpcClient::builder().layer(RetryBackoffLayer::new(3, 1000, 100)).http(rpc_url)
    });

    if config.chain.is_optimism() {
        let elf = include_elf!("rsp-client-op").to_vec();
        let block_execution_strategy_factory =
            create_op_block_execution_strategy_factory(&config.genesis);
        let provider = rpc_client.map(RootProvider::<Optimism>::new);

        let mut executor = build_executor(
            elf,
            provider,
            block_execution_strategy_factory,
            persist_execution_report,
            config,
        )?;

        executor.execute(block_number).await?;
    } else {
        let elf = include_elf!("rsp-client").to_vec();
        let block_execution_strategy_factory =
            create_eth_block_execution_strategy_factory(&config.genesis, config.custom_beneficiary);
        let provider = rpc_client.map(RootProvider::<Ethereum>::new);

        let mut executor = build_executor(
            elf,
            provider,
            block_execution_strategy_factory,
            persist_execution_report,
            config,
        )?;

        executor.execute(block_number).await?;
    }

    Ok(())
}
