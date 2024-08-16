use alloy_provider::ReqwestProvider;
use clap::Parser;
use reth_primitives::B256;
use rsp_client_executor::ChainVariant;
use rsp_host_executor::HostExecutor;
use sp1_sdk::{ProverClient, SP1Stdin};
use std::path::PathBuf;
use tracing_subscriber::{
    filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};
use url::Url;

mod execute;
use execute::process_execution_report;

/// The arguments for the host executable.
#[derive(Debug, Clone, Parser)]
struct HostArgs {
    /// The block number of the block to execute.
    #[clap(long)]
    block_number: u64,
    /// The rpc url used to fetch data about the block. If not provided, will use the
    /// RPC_{chain_id} env var.
    #[clap(long)]
    rpc_url: Option<Url>,
    /// The chain ID. If not provided, requires the rpc_url argument to be provided.
    #[clap(long)]
    chain_id: Option<u64>,
    /// Whether to generate a proof or just execute the block.
    #[clap(long)]
    prove: bool,
    /// The path to the CSV file containing the execution data.
    #[clap(long, default_value = "report.csv")]
    report_path: PathBuf,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Intialize the environment variables.
    dotenv::dotenv().ok();

    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    // Initialize the logger.
    tracing_subscriber::registry().with(fmt::layer()).with(EnvFilter::from_default_env()).init();

    // Parse the command line arguments.
    let args = HostArgs::parse();

    let rpc_url = if let Some(rpc_url) = args.rpc_url {
        rpc_url
    } else {
        let chain_id = args.chain_id.expect("If rpc_url is not provided, chain_id must be.");
        let env_var_key =
            std::env::var(format!("RPC_{}", chain_id)).expect("Could not find RPC_{} in .env");
        let rpc_url = Url::parse(env_var_key.as_str()).expect("invalid rpc url");
        rpc_url
    };

    // Setup the provider.
    let provider = ReqwestProvider::new_http(rpc_url);

    // Setup the host executor.
    let host_executor = HostExecutor::new(provider);

    // Execute the host.
    let (client_input, variant) =
        host_executor.execute(args.block_number).await.expect("failed to execute host");

    // Generate the proof.
    let client = ProverClient::new();

    // Setup the proving key and verification key.
    let (pk, vk) = client.setup(match variant {
        ChainVariant::Ethereum => {
            include_bytes!("../../client-eth/elf/riscv32im-succinct-zkvm-elf")
        }
        ChainVariant::Optimism => include_bytes!("../../client-op/elf/riscv32im-succinct-zkvm-elf"),
    });

    // Execute the block inside the zkVM.
    let mut stdin = SP1Stdin::new();
    let buffer = bincode::serialize(&client_input).unwrap();
    stdin.write_vec(buffer);

    // Only execute the program.
    let (mut public_values, execution_report) =
        client.execute(&pk.elf, stdin.clone()).run().unwrap();

    // Read the block hash.
    let block_hash = public_values.read::<B256>();
    println!("success: block_hash={block_hash}");

    // Process the execute report, print it out, and save data to a CSV specified by
    // report_path.
    process_execution_report(variant, client_input, execution_report, args.report_path)?;

    if args.prove {
        // Actually generate the proof. It is strongly recommended you use the network prover
        // given the size of these programs.
        println!("Starting proof generation.");
        let proof = client.prove(&pk, stdin).compressed().run().expect("Proving should work.");
        println!("Proof generation finished.");

        client.verify(&proof, &vk).expect("proof verification should succeed");
    }

    Ok(())
}
