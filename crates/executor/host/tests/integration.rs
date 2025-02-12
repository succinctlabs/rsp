use std::{fs, sync::Arc};

use alloy_genesis::Genesis;
use alloy_provider::{network::Ethereum, Network, RootProvider};
use reth_chainspec::ChainSpec;
use reth_evm::execute::BlockExecutionStrategyFactory;
use revm_primitives::{address, Address};
use rsp_client_executor::{
    executor::{ClientExecutor, EthClientExecutor},
    io::ClientExecutorInput,
    FromInput, IntoInput, IntoPrimitives,
};
use rsp_host_executor::{EthHostExecutor, HostExecutor};
use rsp_rpc_db::RpcDb;
use serde::{de::DeserializeOwned, Serialize};
use tracing_subscriber::{
    fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt, EnvFilter,
};
use url::Url;

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_ethereum() {
    let genesis_path = fs::canonicalize("../../../bin/host/genesis/1.json").unwrap();

    run_eth_e2e(fs::read_to_string(genesis_path).unwrap(), "RPC_1", 18884864, None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_optimism() {
    let genesis_path = fs::canonicalize("../../../bin/host/genesis/10.json").unwrap();
    let genesis_json = fs::read_to_string(genesis_path).unwrap();
    let genesis = serde_json::from_str::<Genesis>(&genesis_json).unwrap();
    let chain_spec = Arc::new(reth_optimism_chainspec::OpChainSpec::from_genesis(genesis));

    // Setup the host executor.
    let host_executor = rsp_host_executor::OpHostExecutor::optimism(chain_spec.clone());

    // Setup the client executor.
    let client_executor = rsp_client_executor::executor::OpClientExecutor::optimism(chain_spec);

    run_e2e::<_, op_alloy_network::Optimism>(
        host_executor,
        client_executor,
        "RPC_10",
        122853660,
        genesis_json,
        None,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_linea() {
    let genesis_path = fs::canonicalize("../../../bin/host/genesis/59144.json").unwrap();
    run_eth_e2e(
        fs::read_to_string(genesis_path).unwrap(),
        "RPC_59144",
        5600000,
        Some(address!("8f81e2e3f8b46467523463835f965ffe476e1c9e")),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_sepolia() {
    let genesis_path = fs::canonicalize("../../../bin/host/genesis/11155111.json").unwrap();

    run_eth_e2e(fs::read_to_string(genesis_path).unwrap(), "RPC_11155111", 6804324, None).await;
}

async fn run_eth_e2e(
    genesis_json: String,
    env_var_key: &str,
    block_number: u64,
    custom_beneficiary: Option<Address>,
) {
    let genesis = serde_json::from_str::<Genesis>(&genesis_json).unwrap();

    let chain_spec = Arc::<ChainSpec>::new(genesis.into());

    // Setup the host executor.
    let host_executor = EthHostExecutor::eth(chain_spec.clone(), custom_beneficiary);

    // Setup the client executor.
    let client_executor = EthClientExecutor::eth(chain_spec, custom_beneficiary);

    run_e2e::<_, Ethereum>(
        host_executor,
        client_executor,
        env_var_key,
        block_number,
        genesis_json,
        custom_beneficiary,
    )
    .await;
}

async fn run_e2e<F, N>(
    host_executor: HostExecutor<F>,
    client_executor: ClientExecutor<F>,
    env_var_key: &str,
    block_number: u64,
    genesis_json: String,
    custom_beneficiary: Option<Address>,
) where
    F: BlockExecutionStrategyFactory,
    F::Primitives: FromInput + IntoPrimitives<N> + IntoInput + Serialize + DeserializeOwned,
    N: Network,
{
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
    let provider = RootProvider::<N>::new_http(rpc_url);

    let rpc_db = RpcDb::new(provider.clone(), block_number - 1);

    // Execute the host.
    let client_input = host_executor
        .execute(block_number, &rpc_db, &provider, genesis_json, custom_beneficiary)
        .await
        .expect("failed to execute host");

    // Execute the client.
    client_executor.execute(client_input.clone()).expect("failed to execute client");

    // Save the client input to a buffer.
    let buffer = bincode::serialize(&client_input).unwrap();

    // Load the client input from a buffer.
    let _: ClientExecutorInput<F::Primitives> = bincode::deserialize(&buffer).unwrap();
}
