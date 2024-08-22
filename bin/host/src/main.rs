use alloy_provider::{network::AnyNetwork, Provider, ReqwestProvider};
use clap::Parser;
use reth_primitives::B256;
use rsp_client_executor::{
    io::ClientExecutorInput, ChainVariant, CHAIN_ID_ETH_MAINNET, CHAIN_ID_OP_MAINNET,
};
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
    /// Optional path to the directory containing cached client input. A new cache file will be
    /// created from RPC data if it doesn't already exist.
    #[clap(long)]
    cache_dir: Option<PathBuf>,
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

    // We don't need RPC when using cache with known chain ID, so we leave it as `Option<Url>` here
    // and decide on whether to panic later.
    //
    // On the other hand chain ID is always needed.
    let (rpc_url, chain_id) = match (args.rpc_url, args.chain_id) {
        (Some(rpc_url), Some(chain_id)) => (Some(rpc_url), chain_id),
        (None, Some(chain_id)) => {
            match std::env::var(format!("RPC_{}", chain_id)) {
                Ok(rpc_env_var) => {
                    // We don't always need it but if the value exists it has to be valid.
                    (Some(Url::parse(rpc_env_var.as_str()).expect("invalid rpc url")), chain_id)
                }
                Err(_) => {
                    // Not having RPC is okay because we know chain ID.
                    (None, chain_id)
                }
            }
        }
        (Some(rpc_url), None) => {
            // We can find out about chain ID from RPC.
            let provider: ReqwestProvider<AnyNetwork> = ReqwestProvider::new_http(rpc_url.clone());
            let chain_id = provider.get_chain_id().await?;

            (Some(rpc_url), chain_id)
        }
        (None, None) => {
            eyre::bail!("either --rpc-url or --chain-id must be used")
        }
    };

    let variant = match chain_id {
        CHAIN_ID_ETH_MAINNET => ChainVariant::Ethereum,
        CHAIN_ID_OP_MAINNET => ChainVariant::Optimism,
        _ => {
            eyre::bail!("unknown chain ID: {}", chain_id);
        }
    };

    let client_input_from_cache = if let Some(cache_dir) = args.cache_dir.as_ref() {
        let cache_path = cache_dir.join(format!("input/{}/{}.bin", chain_id, args.block_number));

        if cache_path.exists() {
            // TODO: prune the cache if invalid instead
            let mut cache_file = std::fs::File::open(cache_path)?;
            let client_input: ClientExecutorInput = bincode::deserialize_from(&mut cache_file)?;

            Some(client_input)
        } else {
            None
        }
    } else {
        None
    };

    let client_input = match (client_input_from_cache, rpc_url) {
        (Some(client_input_from_cache), _) => client_input_from_cache,
        (None, Some(rpc_url)) => {
            // Cache not found but we have RPC
            // Setup the provider.
            let provider = ReqwestProvider::new_http(rpc_url);

            // Setup the host executor.
            let host_executor = HostExecutor::new(provider);

            // Execute the host.
            let client_input = host_executor
                .execute(args.block_number, variant)
                .await
                .expect("failed to execute host");

            if let Some(cache_dir) = args.cache_dir {
                let input_folder = cache_dir.join(format!("input/{}", chain_id));
                if !input_folder.exists() {
                    std::fs::create_dir_all(&input_folder)?;
                }

                let input_path = input_folder.join(format!("{}.bin", args.block_number));
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
