use alloy_provider::ReqwestProvider;
use clap::Parser;
use reth_primitives::B256;
use rsp_host_executor::{ChainVariant, HostExecutor};
use sp1_sdk::{ProverClient, SP1Stdin};
use tracing_subscriber::{
    filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};

/// The arguments for the host executable.
#[derive(Debug, Clone, Parser)]
struct HostArgs {
    /// The block number of the block to execute.
    #[clap(long)]
    block_number: u64,
    /// The rpc url used to fetch data about the block.
    #[clap(long)]
    rpc_url: String,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    // Initialize the logger.
    tracing_subscriber::registry().with(fmt::layer()).with(EnvFilter::from_default_env()).init();

    // Parse the command line arguments.
    let args = HostArgs::parse();

    // Setup the provider.
    let provider = ReqwestProvider::new_http(args.rpc_url.parse()?);

    // Setup the host executor.
    let host_executor = HostExecutor::new(provider);

    // Execute the host.
    let (client_input, variant) =
        host_executor.execute(args.block_number).await.expect("failed to execute host");

    // Generate the proof.
    let client = ProverClient::new();

    // Setup the proving key and verification key.
    let (pk, _) = client.setup(match variant {
        ChainVariant::Ethereum => {
            include_bytes!("../../client-eth/elf/riscv32im-succinct-zkvm-elf")
        }
        ChainVariant::Optimism => include_bytes!("../../client-op/elf/riscv32im-succinct-zkvm-elf"),
    });

    // Execute the block inside the zkVM.
    let mut stdin = SP1Stdin::new();
    let buffer = bincode::serialize(&client_input).unwrap();
    stdin.write_vec(buffer);
    let (mut public_values, _) = client.execute(&pk.elf, stdin).run().unwrap();

    // Read the block hash.
    let block_hash = public_values.read::<B256>();
    println!("success: block_hash={block_hash}");

    Ok(())
}
