use alloy_consensus::BlockHeader;
use csv::WriterBuilder;
use reth_primitives::NodePrimitives;
use reth_primitives_traits::BlockBody;
use rsp_client_executor::io::ClientExecutorInput;
use rsp_host_executor::ExecutionHooks;
use serde::{Deserialize, Serialize};
use sp1_core_executor::syscalls::SyscallCode;
use sp1_sdk::ExecutionReport;
use std::{fs::OpenOptions, path::PathBuf};

#[derive(Serialize, Deserialize)]
struct ExecutionReportData {
    chain_id: u64,
    block_number: u64,
    gas_used: u64,
    tx_count: usize,
    number_cycles: u64,
    number_syscalls: u64,
    bn_add_cycles: u64,
    bn_mul_cycles: u64,
    bn_pair_cycles: u64,
    kzg_point_eval_cycles: u64,
    keccak_count: u64,
    secp256k1_decompress_count: u64,
}

#[derive(Debug)]
pub struct PersistExecutionReport {
    chain_id: u64,
    report_path: PathBuf,
}

impl PersistExecutionReport {
    pub fn new(chain_id: u64, report_path: PathBuf) -> Self {
        Self { chain_id, report_path }
    }
}

impl ExecutionHooks for PersistExecutionReport {
    async fn on_execution_end<P: NodePrimitives>(
        &self,
        _block_number: u64,
        client_input: &ClientExecutorInput<P>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        println!("\nExecution report:\n{}", execution_report);

        let block_number = client_input.current_block.header.number();
        let gas_used = client_input.current_block.header.gas_used();
        let tx_count = client_input.current_block.body.transaction_count();
        let number_cycles = execution_report.total_instruction_count();
        let number_syscalls = execution_report.total_syscall_count();

        let bn_add_cycles = *execution_report.cycle_tracker.get("precompile-bn-add").unwrap_or(&0);
        let bn_mul_cycles = *execution_report.cycle_tracker.get("precompile-bn-mul").unwrap_or(&0);
        let bn_pair_cycles =
            *execution_report.cycle_tracker.get("precompile-bn-pair").unwrap_or(&0);
        let kzg_point_eval_cycles =
            *execution_report.cycle_tracker.get("precompile-kzg-point-evaluation").unwrap_or(&0);
        let keccak_count = execution_report.syscall_counts[SyscallCode::KECCAK_PERMUTE];
        let secp256k1_decompress_count =
            execution_report.syscall_counts[SyscallCode::SECP256K1_DECOMPRESS];

        let report_data = ExecutionReportData {
            chain_id: self.chain_id,
            block_number,
            gas_used,
            tx_count,
            number_cycles,
            number_syscalls,
            bn_add_cycles,
            bn_mul_cycles,
            bn_pair_cycles,
            kzg_point_eval_cycles,
            keccak_count,
            secp256k1_decompress_count,
        };

        // Open the file for appending or create it if it doesn't exist
        let file = OpenOptions::new().append(true).create(true).open(self.report_path.clone())?;

        // Check if the file is empty
        let file_is_empty = file.metadata()?.len() == 0;

        let mut writer = WriterBuilder::new().has_headers(file_is_empty).from_writer(file);
        writer.serialize(report_data)?;
        writer.flush()?;

        Ok(())
    }
}
