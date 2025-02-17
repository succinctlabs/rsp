#![warn(unused_crate_dependencies)]

use clap::Parser;
use execute::PersistExecutionReport;
use rsp_host_executor::{
    create_eth_block_execution_strategy_factory, create_op_block_execution_strategy_factory,
    FullExecutor,
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
    let config = args.into_config().await?;
    let persist_execution_report = PersistExecutionReport::new(config.chain.id(), report_path);

    if config.chain.is_optimism() {
        let elf = include_elf!("rsp-client-op").to_vec();
        let block_execution_strategy_factory =
            create_op_block_execution_strategy_factory(&config.genesis);

        let mut executor = FullExecutor::new(
            elf,
            block_execution_strategy_factory,
            persist_execution_report,
            config,
        );

        executor.execute(block_number).await?;
    } else {
        let elf = include_elf!("rsp-client").to_vec();
        let block_execution_strategy_factory =
            create_eth_block_execution_strategy_factory(&config.genesis, config.custom_beneficiary);

        let mut executor = FullExecutor::new(
            elf,
            block_execution_strategy_factory,
            persist_execution_report,
            config,
        );

        executor.execute(block_number).await?;
    }

    Ok(())
}
