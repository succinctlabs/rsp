use alloy_provider::{Provider, ProviderBuilder, ReqwestProvider};
use alloy_transport_ws::WsConnect;
use clap::Parser;
use eth_proofs_client::EthProofsClient;
use futures::{future::ready, StreamExt};
use reth_primitives::B256;
use rsp_client_executor::ChainVariant;
use rsp_host_executor::HostExecutor;
use sp1_sdk::{include_elf, ProverClient, SP1Stdin};
use tracing::info;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use url::Url;

mod eth_proofs_client;

/// The arguments for the cli.
#[derive(Debug, Clone, Parser)]
struct Args {
    /// The HTTP rpc url used to fetch data about the block.
    #[clap(long)]
    http_rpc_url: Url,

    /// The WS rpc url used to fetch data about the block.
    #[clap(long)]
    ws_rpc_url: Url,

    /// The interval at which to execute blocks.
    #[clap(long, default_value_t = 100)]
    block_interval: u64,

    /// ETH proofs endpoint.
    #[clap(long, env)]
    eth_proofs_endpoint: String,

    /// ETH proofs API token.
    #[clap(long, env)]
    eth_proofs_api_token: String,

    /// Optional ETH proofs cluster ID.
    #[clap(long, default_value_t = 1)]
    eth_proofs_cluster_id: u64,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    // Initialize the logger.
    tracing_subscriber::registry().with(fmt::layer()).with(EnvFilter::from_default_env()).init();

    // Parse the command line arguments.
    let args = Args::parse();

    let eth_proofs_client = EthProofsClient::new(
        args.eth_proofs_cluster_id,
        args.eth_proofs_endpoint,
        args.eth_proofs_api_token,
    );

    let ws = WsConnect::new(args.ws_rpc_url);
    let ws_provider = ProviderBuilder::new().on_ws(ws).await?;
    let http_provider = ReqwestProvider::new_http(args.http_rpc_url);

    // Setup the host executor.
    let host_executor = HostExecutor::new(http_provider.clone());
    let client = ProverClient::from_env();
    let variant = ChainVariant::mainnet();

    // Setup the proving key and verification key.
    let (pk, vk) = client.setup(include_elf!("rsp-client-eth"));

    // Subscribe to block headers.
    let subscription = ws_provider.subscribe_blocks().await?;
    let mut stream =
        subscription.into_stream().filter(|b| ready(b.header.number % args.block_interval == 0));

    info!("Latest block number: {}", http_provider.get_block_number().await?);

    while let Some(block) = stream.next().await {
        let block_number = block.header.number;

        //Report we stated working on the block.
        eth_proofs_client.queued(block_number).await?;

        info!("Executing block {block_number} in the host");

        // Execute the host.
        let client_input = host_executor.execute(block_number, &variant, None).await?;

        info!("Executing block {block_number} inside the zkVM");

        // Execute the block inside the zkVM.
        let mut stdin = SP1Stdin::new();
        let buffer = bincode::serialize(&client_input).unwrap();
        stdin.write_vec(buffer);

        // Only execute the program.
        let (mut public_values, execution_report) = client.execute(&pk.elf, &stdin).run().unwrap();

        // Read the block hash.
        let block_hash = public_values.read::<B256>();
        info!("Success! block hash: {block_hash}");

        // Report we stated proving.
        eth_proofs_client.proving(block_number).await?;

        info!("Starting proof generation.");

        let start = std::time::Instant::now();
        let proof = client.prove(&pk, &stdin).compressed().run().expect("Proving should work.");
        let proof_bytes = bincode::serialize(&proof.proof).unwrap();
        let elapsed = start.elapsed().as_secs_f32();

        // Report we proved.
        eth_proofs_client
            .proved(&proof_bytes, block_number, &execution_report, elapsed, &vk)
            .await?;

        info!("Block {block_number} proved in {elapsed} seconds.");
    }

    Ok(())
}
