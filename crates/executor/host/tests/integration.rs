use std::{fs, path::PathBuf};

use alloy_provider::ReqwestProvider;
use rsp_client_executor::{io::ClientExecutorInput, ChainVariant, ClientExecutor};
use rsp_host_executor::HostExecutor;
use tracing_subscriber::{
    fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt, EnvFilter,
};
use url::Url;

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_ethereum() {
    run_e2e(ChainVariant::mainnet(), "RPC_1", 18884864, None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_optimism() {
    run_e2e(ChainVariant::op_mainnet(), "RPC_10", 122853660, None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_linea() {
    run_e2e(ChainVariant::linea_mainnet(), "RPC_59144", 5600000, None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_sepolia() {
    let genesis_path = fs::canonicalize("./tests/fixtures/sepolia_genesis.json").unwrap();

    run_e2e(
        ChainVariant::from_genesis_path(&genesis_path).unwrap(),
        "RPC_11155111",
        6804324,
        Some(genesis_path),
    )
    .await;
}

async fn run_e2e(
    variant: ChainVariant,
    env_var_key: &str,
    block_number: u64,
    genesis_path: Option<PathBuf>,
) {
    // Intialize the environment variables.
    dotenv::dotenv().ok();

    // Initialize the logger.
    let _ = tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .try_init();

    // Setup the provider.
    let rpc_url =
        Url::parse(std::env::var(env_var_key).unwrap().as_str()).expect("invalid rpc url");
    let provider = ReqwestProvider::new_http(rpc_url);

    // Setup the host executor.
    let host_executor = HostExecutor::new(provider);

    // Execute the host.
    let client_input = host_executor
        .execute(block_number, &variant, genesis_path)
        .await
        .expect("failed to execute host");

    // Setup the client executor.
    let client_executor = ClientExecutor;

    // Execute the client.
    client_executor.execute(client_input.clone(), &variant).expect("failed to execute client");

    // Save the client input to a buffer.
    let buffer = bincode::serialize(&client_input).unwrap();

    // Load the client input from a buffer.
    let _: ClientExecutorInput = bincode::deserialize(&buffer).unwrap();
}
