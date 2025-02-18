use std::{env, fs::File, io::Write};

use alloy_chains::Chain;
use alloy_network::Ethereum;
use alloy_provider::RootProvider;
use reth_primitives::NodePrimitives;
use rsp_client_executor::io::ClientExecutorInput;
use rsp_host_executor::{
    build_executor, create_eth_block_execution_strategy_factory, BlockExecutor, Config,
    ExecutionHooks,
};
use rsp_primitives::genesis::Genesis;
use sp1_sdk::{include_elf, ExecutionReport};
use thousands::Separable;
use url::Url;

#[tokio::test(flavor = "multi_thread")]
async fn test_in_zkvm() {
    // Intialize the environment variables.
    dotenv::dotenv().ok();

    let config = Config {
        chain: Chain::mainnet(),
        genesis: Genesis::Mainnet,
        cache_dir: None,
        custom_beneficiary: None,
        prove: false,
    };

    let rpc_url = Url::parse(env::var("RPC_1").unwrap().as_str()).expect("invalid rpc url");
    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, config.custom_beneficiary);

    let provider = RootProvider::<Ethereum>::new_http(rpc_url);

    let mut executor = build_executor(
        elf,
        Some(provider),
        block_execution_strategy_factory,
        ExecutionSummary,
        config,
    )
    .unwrap();

    executor.execute(20600000).await.unwrap();
}

pub struct ExecutionSummary;

impl ExecutionHooks for ExecutionSummary {
    async fn on_execution_end<P: NodePrimitives>(
        &self,
        _block_number: u64,
        _client_input: &ClientExecutorInput<P>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        let path = env::var("GITHUB_OUTPUT")?;
        let mut file = File::options().create(true).append(true).open(path)?;

        writeln!(
            file,
            "CYCLE_COUNT={}",
            execution_report.total_instruction_count().separate_with_commas()
        )?;

        Ok(())
    }
}
