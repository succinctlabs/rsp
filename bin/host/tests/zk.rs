use std::{env, fs::File, io::Write};

use alloy_chains::Chain;
use alloy_consensus::Block;
use alloy_network::Ethereum;
use alloy_provider::RootProvider;
use madato::{mk_table, types::TableRow};
use reth_primitives::NodePrimitives;
use rsp_host_executor::{
    build_executor, create_eth_block_execution_strategy_factory, BlockExecutor, Config,
    ExecutionHooks,
};
use rsp_primitives::genesis::Genesis;
use serde::{Deserialize, Serialize};
use sp1_sdk::{include_elf, ExecutionReport};
use thousands::Separable;
use url::Url;

#[tokio::test(flavor = "multi_thread")]
async fn test_in_zkvm() {
    // Intialize the environment variables.
    dotenv::dotenv().ok();

    let is_base_branch = env::var("BASE_BRANCH").is_ok();

    let config = Config {
        chain: Chain::mainnet(),
        genesis: Genesis::Mainnet,
        rpc_url: None,
        cache_dir: None,
        custom_beneficiary: None,
        prove: false,
        opcode_tracking: false,
    };

    let rpc_url = Url::parse(env::var("RPC_1").unwrap().as_str()).expect("invalid rpc url");
    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, config.custom_beneficiary);

    let provider = RootProvider::<Ethereum>::new_http(rpc_url);

    let executor = build_executor(
        elf,
        Some(provider),
        block_execution_strategy_factory,
        Hook::new(is_base_branch),
        config,
    )
    .await
    .unwrap();

    executor.execute(20600000).await.unwrap();
}

enum Hook {
    WithCurrentDev,
    OnBaseBranch,
}

impl Hook {
    pub fn new(is_base_branch: bool) -> Self {
        if is_base_branch {
            Self::OnBaseBranch
        } else {
            Self::WithCurrentDev
        }
    }
}

impl ExecutionHooks for Hook {
    async fn on_execution_end<P: NodePrimitives>(
        &self,
        executed_block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        match self {
            Hook::WithCurrentDev => {
                let stats = Stats {
                    cycle_count: execution_report.total_instruction_count(),
                    syscall_count: execution_report.total_syscall_count(),
                };

                serde_json::to_writer(File::create("cycle_stats.json")?, &stats)?;
            }
            Hook::OnBaseBranch => {
                let path = env::var("GITHUB_OUTPUT")?;
                let current_dev_stats =
                    serde_json::from_reader::<_, Stats>(File::open("cycle_stats.json")?)?;
                let mut output_file = File::options().create(true).append(true).open(path)?;

                let row = |label: &str, value: String, diff: String| {
                    let mut r = TableRow::new();
                    r.insert("Label".to_string(), label.to_string());
                    r.insert("Value".to_string(), value);
                    r.insert("Diff (%)".to_string(), diff);
                    r
                };

                let diff_percentage =
                    |initial: f64, current: f64| (initial - current) / initial * 100_f64;

                let table = mk_table(
                    &[
                        row("Block Number", executed_block.number.to_string(), String::from("---")),
                        row(
                            "Cycle Count",
                            execution_report.total_instruction_count().separate_with_commas(),
                            format!(
                                "{:.2}",
                                diff_percentage(
                                    execution_report.total_instruction_count() as f64,
                                    current_dev_stats.cycle_count as f64,
                                )
                            ),
                        ),
                        row(
                            "Syscall Count",
                            execution_report.total_syscall_count().separate_with_commas(),
                            format!(
                                "{:.2}",
                                diff_percentage(
                                    execution_report.total_syscall_count() as f64,
                                    current_dev_stats.syscall_count as f64,
                                )
                            ),
                        ),
                    ],
                    &None,
                );

                println!("{table}");

                writeln!(output_file, "EXECUTION_REPORT<<EOF")?;
                writeln!(output_file, "{}", table)?;
                writeln!(output_file, "EOF")?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Stats {
    pub cycle_count: u64,
    pub syscall_count: u64,
}
