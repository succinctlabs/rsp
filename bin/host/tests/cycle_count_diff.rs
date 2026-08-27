use std::{env, fs::File, io::Write, sync::Arc, time::Duration};

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
use sp1_sdk::{include_elf, CpuProver, ExecutionReport};
use thousands::Separable;
use url::Url;

fn required_cycle_count(report: &ExecutionReport, label: &str) -> eyre::Result<u64> {
    report.cycle_tracker.get(label).copied().ok_or_else(|| {
        eyre::eyre!(
            "missing required cycle counter `{label}`; run this test with `--features cycle-tracking`"
        )
    })
}

fn format_diff_percentage(initial: u64, current: u64) -> String {
    if initial == 0 {
        "N/A".to_string()
    } else {
        format!("{:.2}", (current as f64 - initial as f64) / initial as f64 * 100.0)
    }
}

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
        stdin_dir: None,
        custom_beneficiary: None,
        prove_mode: None,
        skip_client_execution: false,
        opcode_tracking: false,
        state_backend: Default::default(),
    };

    let rpc_url = Url::parse(env::var("RPC_1").unwrap().as_str()).expect("invalid rpc url");
    let elf = include_elf!("rsp-client").to_vec();
    let block_execution_strategy_factory =
        create_eth_block_execution_strategy_factory(&config.genesis, config.custom_beneficiary);

    let provider = RootProvider::<Ethereum>::new_http(rpc_url);
    let client = Arc::new(CpuProver::new().await);

    let executor = build_executor::<EthExecutorComponents<_, CpuProver>, _>(
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
        _execution_duration: Duration,
    ) -> eyre::Result<()> {
        match self {
            Hook::WithCurrentDev => {
                let stats = Stats {
                    total_cycle_count: execution_report.total_instruction_count(),
                    deserialize_inputs: required_cycle_count(execution_report, DESERIALZE_INPUTS)?,
                    initialize_witness_db_cycles_count: required_cycle_count(
                        execution_report,
                        INIT_WITNESS_DB,
                    )?,
                    recover_senders_cycles_count: required_cycle_count(
                        execution_report,
                        RECOVER_SENDERS,
                    )?,
                    header_validation_cycles_count: required_cycle_count(
                        execution_report,
                        VALIDATE_HEADER,
                    )?,
                    block_execution_cycles_count: required_cycle_count(
                        execution_report,
                        BLOCK_EXECUTION,
                    )?,
                    block_validation_cycles_count: required_cycle_count(
                        execution_report,
                        VALIDATE_EXECUTION,
                    )?,
                    state_root_computation_cycles_count: required_cycle_count(
                        execution_report,
                        COMPUTE_STATE_ROOT,
                    )?,
                    syscall_count: execution_report.total_syscall_count(),
                    prover_gas: execution_report.gas().unwrap_or_default(),
                };

                serde_json::to_writer(File::create("cycle_stats.json")?, &stats)?;
            }
            Hook::OnBaseBranch => {
                let path = env::var("GITHUB_OUTPUT")?;
                let current_dev_stats =
                    serde_json::from_reader::<_, Stats>(File::open("cycle_stats.json")?)?;
                let mut output_file = File::options().create(true).append(true).open(path)?;

                let row = |label: &str, initial: u64, current: u64| {
                    let mut r = TableRow::new();

                    r.insert(format!("Block {}", executed_block.number), label.to_string());
                    r.insert("Base Branch".to_string(), initial.separate_with_commas());
                    r.insert("Current PR".to_string(), current.separate_with_commas());
                    r.insert(
                        "Diff".to_string(),
                        (current as i64 - initial as i64).separate_with_commas(),
                    );
                    r.insert("Diff (%)".to_string(), format_diff_percentage(initial, current));
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
                            required_cycle_count(execution_report, DESERIALZE_INPUTS)?,
                            current_dev_stats.deserialize_inputs,
                        ),
                        row(
                            "Initialize Witness DB",
                            required_cycle_count(execution_report, INIT_WITNESS_DB)?,
                            current_dev_stats.initialize_witness_db_cycles_count,
                        ),
                        row(
                            "Recover Senders",
                            required_cycle_count(execution_report, RECOVER_SENDERS)?,
                            current_dev_stats.recover_senders_cycles_count,
                        ),
                        row(
                            "Header Validation",
                            required_cycle_count(execution_report, VALIDATE_HEADER)?,
                            current_dev_stats.header_validation_cycles_count,
                        ),
                        row(
                            "Block Execution",
                            required_cycle_count(execution_report, BLOCK_EXECUTION)?,
                            current_dev_stats.block_execution_cycles_count,
                        ),
                        row(
                            "Block Validation",
                            required_cycle_count(execution_report, VALIDATE_EXECUTION)?,
                            current_dev_stats.block_validation_cycles_count,
                        ),
                        row(
                            "State Root Computation",
                            required_cycle_count(execution_report, COMPUTE_STATE_ROOT)?,
                            current_dev_stats.state_root_computation_cycles_count,
                        ),
                        row(
                            "Syscall Count",
                            execution_report.total_syscall_count(),
                            current_dev_stats.syscall_count,
                        ),
                        row(
                            "Prover Gas",
                            execution_report.gas().unwrap_or_default(),
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

#[test]
fn missing_required_cycle_count_is_an_error() {
    let error = required_cycle_count(&ExecutionReport::default(), DESERIALZE_INPUTS).unwrap_err();

    assert!(error.to_string().contains(DESERIALZE_INPUTS));
}

#[test]
fn zero_baseline_percentage_is_not_available() {
    assert_eq!(format_diff_percentage(0, 0), "N/A");
    assert_eq!(format_diff_percentage(100, 105), "5.00");
}
