use std::sync::Arc;

use alloy_provider::{network::Ethereum, Network, RootProvider};
use reth_chainspec::ChainSpec;
use reth_evm::execute::BlockExecutionStrategyFactory;
use reth_optimism_chainspec::OpChainSpec;
use revm_primitives::{address, Address};
use rsp_client_executor::{
    executor::{ClientExecutor, EthClientExecutor},
    io::ClientExecutorInput,
    FromInput, IntoInput, IntoPrimitives, ValidateBlockPostExecution,
};
use rsp_host_executor::{EthHostExecutor, HostExecutor};
use rsp_primitives::genesis::Genesis;
use rsp_rpc_db::RpcDb;
use serde::{de::DeserializeOwned, Serialize};
use tracing_subscriber::{
    fmt, prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt, EnvFilter,
};
use url::Url;

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_ethereum() {
    run_eth_e2e(&Genesis::Mainnet, "RPC_1", 18884864, None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_optimism() {
    let chain_spec: Arc<OpChainSpec> = Arc::new((&Genesis::OpMainnet).try_into().unwrap());

    // Setup the host executor.
    let host_executor = rsp_host_executor::OpHostExecutor::optimism(chain_spec.clone());

    // Setup the client executor.
    let client_executor = rsp_client_executor::executor::OpClientExecutor::optimism(chain_spec);

    run_e2e::<_, op_alloy_network::Optimism>(
        host_executor,
        client_executor,
        "RPC_10",
        122853660,
        &Genesis::OpMainnet,
        None,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_linea() {
    run_eth_e2e(
        &Genesis::Linea,
        "RPC_59144",
        5600000,
        Some(address!("8f81e2e3f8b46467523463835f965ffe476e1c9e")),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_e2e_sepolia() {
    run_eth_e2e(&Genesis::Sepolia, "RPC_11155111", 6804324, None).await;
}

async fn run_eth_e2e(
    genesis: &Genesis,
    env_var_key: &str,
    block_number: u64,
    custom_beneficiary: Option<Address>,
) {
    let chain_spec: Arc<ChainSpec> = Arc::new(genesis.try_into().unwrap());

    // Setup the host executor.
    let host_executor = EthHostExecutor::eth(chain_spec.clone(), custom_beneficiary);

    // Setup the client executor.
    let client_executor = EthClientExecutor::eth(chain_spec, custom_beneficiary);

    run_e2e::<_, Ethereum>(
        host_executor,
        client_executor,
        env_var_key,
        block_number,
        genesis,
        custom_beneficiary,
    )
    .await;
}

async fn run_e2e<F, N>(
    host_executor: HostExecutor<F>,
    client_executor: ClientExecutor<F>,
    env_var_key: &str,
    block_number: u64,
    genesis: &Genesis,
    custom_beneficiary: Option<Address>,
) where
    F: BlockExecutionStrategyFactory,
    F::Primitives: FromInput
        + IntoPrimitives<N>
        + IntoInput
        + ValidateBlockPostExecution
        + Serialize
        + DeserializeOwned,
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
        .execute(block_number, &rpc_db, &provider, genesis.clone(), custom_beneficiary, false)
        .await
        .expect("failed to execute host");

    // Execute the client.
    client_executor.execute(client_input.clone()).expect("failed to execute client");

    // Save the client input to a buffer.
    let buffer = bincode::serialize(&client_input).unwrap();

    // Load the client input from a buffer.
    let _: ClientExecutorInput<F::Primitives> = bincode::deserialize(&buffer).unwrap();
}
