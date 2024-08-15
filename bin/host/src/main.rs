use alloy_provider::ReqwestProvider;
use clap::Parser;
use csv::Writer;
use reth_primitives::B256;
use rsp_client_executor::ChainVariant;
use rsp_host_executor::HostExecutor;
use sp1_sdk::{ProverClient, SP1Stdin};
use std::{fs::OpenOptions, path::Path};
use tracing_subscriber::{
    filter::EnvFilter, fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt,
};
use url::Url;

/// The arguments for the host executable.
#[derive(Debug, Clone, Parser)]
struct HostArgs {
    /// The block number of the block to execute.
    #[clap(long)]
    block_number: u64,
    /// The rpc url used to fetch data about the block. If not provided, will use the
    /// RPC_{chain_id} env var.
    #[clap(long)]
    rpc_url: Option<String>,
    /// The chain ID. If not provided, requires the rpc_url argument to be provided.
    #[clap(long)]
    chain_id: Option<u64>,
    /// Whether to generate a proof or just execute the block.
    proof: bool,
    /// The path to the CSV file containing the execution data.
    #[clap(long)]
    report_path: Option<String>,
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

    let rpc_url = if args.rpc_url.is_none() {
        let chain_id = args.chain_id.expect("If rpc_url is not provided, chain_id must be.");
        let env_var_key =
            std::env::var(format!("RPC_{}", chain_id)).expect("Could not find RPC_{} in .env");
        let rpc_url = Url::parse(env_var_key.as_str()).expect("invalid rpc url");
        rpc_url
    } else {
        let rpc_url = args.rpc_url.unwrap();
        rpc_url.parse()?
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

    if !args.proof {
        // Only execute the program.
        let (mut public_values, execution_report) = client.execute(&pk.elf, stdin).run().unwrap();

        // Read the block hash.
        let block_hash = public_values.read::<B256>();
        println!("success: block_hash={block_hash}");

        println!("\nExecution report:\n{}", execution_report);

        let report_path = args.report_path.unwrap_or("report.csv".to_string());

        let chain_id = variant.chain_id();
        let executed_block = client_input.current_block;
        let block_number = executed_block.header.number;
        let gas_used = executed_block.header.gas_used;
        let tx_count = executed_block.body.len();
        let number_cycles = execution_report.total_instruction_count();
        let number_syscalls = execution_report.total_syscall_count();

        let bn_add_cycles = execution_report.cycle_tracker.get("precompile-bn-add").unwrap_or(&0);
        let bn_mul_cycles = execution_report.cycle_tracker.get("precompile-bn-mul").unwrap_or(&0);
        let bn_pair_cycles = execution_report.cycle_tracker.get("precompile-bn-pair").unwrap_or(&0);

        // TODO: we can track individual syscalls in our CSV once we have sp1-core as a dependency
        // let keccak_count = execution_report.syscall_counts.get(SyscallCode::KECCAK_PERMUTE);
        // let secp256k1_decompress_count =
        //     execution_report.syscall_counts.get(SyscallCode::SECP256K1_DECOMPRESS);

        // Check if the file exists
        let file_exists = Path::new(&report_path).exists();

        // Open the file for appending or create it if it doesn't exist
        let file = OpenOptions::new().append(true).create(true).open(report_path)?;

        let mut writer = Writer::from_writer(file);

        // Write the header if the file doesn't exist
        if !file_exists {
            writer.write_record([
                "chain_id",
                "block_number",
                "gas_used",
                "tx_count",
                "number_cycles",
                "number_syscalls",
                "bn_add_cycles",
                "bn_mul_cycles",
                "bn_pair_cycles",
            ])?;
        }

        // Write the data
        writer.write_record(&[
            chain_id.to_string(),
            block_number.to_string(),
            gas_used.to_string(),
            tx_count.to_string(),
            number_cycles.to_string(),
            number_syscalls.to_string(),
            bn_add_cycles.to_string(),
            bn_mul_cycles.to_string(),
            bn_pair_cycles.to_string(),
        ])?;

        writer.flush()?;
    } else {
        unimplemented!("Right now we only support execution, not proof generation");
    }

    Ok(())
}
