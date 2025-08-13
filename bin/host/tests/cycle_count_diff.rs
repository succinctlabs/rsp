use std::{env, fs::File, io::Write, sync::Arc};

use alloy_chains::Chain;
use alloy_consensus::Block;
use alloy_network::Ethereum;
use alloy_provider::RootProvider;
use madato::{mk_table, types::TableRow};
use reth_primitives_traits::NodePrimitives;
use rsp_client_executor::executor::{
    BLOCK_EXECUTION, COMPUTE_STATE_ROOT, DESERIALZE_INPUTS, INIT_WITNESS_DB, RECOVER_SENDERS,
    VALIDATE_EXECUTION, VALIDATE_HEADER,
};
use rsp_host_executor::{
    build_executor, create_eth_block_execution_strategy_factory, BlockExecutor, Config,
    EthExecutorComponents, ExecutionHooks,
};
use rsp_primitives::genesis::Genesis;
use serde::{Deserialize, Serialize};
use sp1_sdk::{include_elf, EnvProver, ExecutionReport};
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
        prove_mode: None,
        skip_client_execution: false,
        opcode_tracking: false,
    };

    let rpc_url = Url::parse(env::var("RPC_1").unwrap().as_str()).expect("invalid rpc url");
    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, config.custom_beneficiary);

    let provider = RootProvider::<Ethereum>::new_http(rpc_url);
    let client = Arc::new(EnvProver::new());

    let executor = build_executor::<EthExecutorComponents<_>, _>(
        elf,
        Some(provider),
        block_execution_strategy_factory,
        client,
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
                    total_cycle_count: execution_report.total_instruction_count(),
                    deserialize_inputs: execution_report
                        .cycle_tracker
                        .get(DESERIALZE_INPUTS)
                        .copied()
                        .unwrap_or(0),
                    initialize_witness_db_cycles_count: execution_report
                        .cycle_tracker
                        .get(INIT_WITNESS_DB)
                        .copied()
                        .unwrap_or(0),
                    recover_senders_cycles_count: execution_report
                        .cycle_tracker
                        .get(RECOVER_SENDERS)
                        .copied()
                        .unwrap_or(0),
                    header_validation_cycles_count: execution_report
                        .cycle_tracker
                        .get(VALIDATE_HEADER)
                        .copied()
                        .unwrap_or(0),
                    block_execution_cycles_count: execution_report
                        .cycle_tracker
                        .get(BLOCK_EXECUTION)
                        .copied()
                        .unwrap_or(0),
                    block_validation_cycles_count: execution_report
                        .cycle_tracker
                        .get(VALIDATE_EXECUTION)
                        .copied()
                        .unwrap_or(0),
                    state_root_computation_cycles_count: execution_report
                        .cycle_tracker
                        .get(COMPUTE_STATE_ROOT)
                        .copied()
                        .unwrap_or(0),
                    syscall_count: execution_report.total_syscall_count(),
                    prover_gas: execution_report.gas.unwrap_or_default(),
                };

                serde_json::to_writer(File::create("cycle_stats.json")?, &stats)?;
            }
            Hook::OnBaseBranch => {
                let path = env::var("GITHUB_OUTPUT")?;
                let current_dev_stats =
                    serde_json::from_reader::<_, Stats>(File::open("cycle_stats.json")?)?;
                let mut output_file = File::options().create(true).append(true).open(path)?;

                let diff_percentage =
                    |initial: f64, current: f64| (initial - current) / initial * -100_f64;

                let row = |label: &str, initial: u64, current: u64| {
                    let mut r = TableRow::new();
                    let diff = format!("{:.2}", diff_percentage(initial as f64, current as f64,));

                    r.insert(format!("Block {}", executed_block.number), label.to_string());
                    r.insert("Base Branch".to_string(), initial.separate_with_commas());
                    r.insert("Current PR".to_string(), current.separate_with_commas());
                    r.insert(
                        "Diff".to_string(),
                        (current as i64 - initial as i64).separate_with_commas(),
                    );
                    r.insert("Diff (%)".to_string(), diff);
                    r
                };

                let table = mk_table(
                    &[
                        row(
                            "Total Cycle Count",
                            execution_report.total_instruction_count(),
                            current_dev_stats.total_cycle_count,
                        ),
                        row(
                            "Inputs deserialization",
                            execution_report
                                .cycle_tracker
                                .get(DESERIALZE_INPUTS)
                                .copied()
                                .unwrap_or_default(),
                            current_dev_stats.deserialize_inputs,
                        ),
                        row(
                            "Initialize Witness DB",
                            execution_report
                                .cycle_tracker
                                .get(INIT_WITNESS_DB)
                                .copied()
                                .unwrap_or_default(),
                            current_dev_stats.initialize_witness_db_cycles_count,
                        ),
                        row(
                            "Recover Senders",
                            execution_report
                                .cycle_tracker
                                .get(RECOVER_SENDERS)
                                .copied()
                                .unwrap_or_default(),
                            current_dev_stats.recover_senders_cycles_count,
                        ),
                        row(
                            "Header Validation",
                            execution_report
                                .cycle_tracker
                                .get(VALIDATE_HEADER)
                                .copied()
                                .unwrap_or_default(),
                            current_dev_stats.header_validation_cycles_count,
                        ),
                        row(
                            "Block Execution",
                            execution_report
                                .cycle_tracker
                                .get(BLOCK_EXECUTION)
                                .copied()
                                .unwrap_or_default(),
                            current_dev_stats.block_execution_cycles_count,
                        ),
                        row(
                            "Block Validation",
                            execution_report
                                .cycle_tracker
                                .get(VALIDATE_EXECUTION)
                                .copied()
                                .unwrap_or_default(),
                            current_dev_stats.block_validation_cycles_count,
                        ),
                        row(
                            "State Root Computation",
                            execution_report
                                .cycle_tracker
                                .get(COMPUTE_STATE_ROOT)
                                .copied()
                                .unwrap_or_default(),
                            current_dev_stats.state_root_computation_cycles_count,
                        ),
                        row(
                            "Syscall Count",
                            execution_report.total_syscall_count(),
                            current_dev_stats.syscall_count,
                        ),
                        row(
                            "Prover Gas",
                            execution_report.gas.unwrap_or_default(),
                            current_dev_stats.prover_gas,
                        ),
                    ],
                    &None,
                );

                println!("{table}");

                writeln!(output_file, "EXECUTION_REPORT<<EOF")?;
                writeln!(output_file, "{table}")?;
                writeln!(output_file, "EOF")?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Stats {
    pub total_cycle_count: u64,
    pub deserialize_inputs: u64,
    pub initialize_witness_db_cycles_count: u64,
    pub recover_senders_cycles_count: u64,
    pub header_validation_cycles_count: u64,
    pub block_execution_cycles_count: u64,
    pub block_validation_cycles_count: u64,
    pub state_root_computation_cycles_count: u64,
    pub syscall_count: u64,
    pub prover_gas: u64,
}
