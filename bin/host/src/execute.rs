use alloy_consensus::{Block, BlockHeader};
use csv::{Writer, WriterBuilder};
use reth_primitives_traits::{BlockBody, NodePrimitives};
use revm_bytecode::opcode::OPCODE_INFO;
use rsp_client_executor::executor::{
    BLOCK_EXECUTION, COMPUTE_STATE_ROOT, DESERIALZE_INPUTS, INIT_WITNESS_DB, RECOVER_SENDERS,
    VALIDATE_EXECUTION,
};
use rsp_host_executor::ExecutionHooks;
use serde::{Deserialize, Serialize};
use sp1_core_executor::syscalls::SyscallCode;
use sp1_sdk::ExecutionReport;
use std::{
    fs::{File, OpenOptions},
    path::PathBuf,
};
use strum::IntoEnumIterator;

const PRECOMPILES: [&str; 10] = [
    "ecrecover",
    "sha256",
    "ripemd160",
    "identity",
    "modexp",
    "bn-add",
    "bn-mul",
    "bn-pair",
    "blake2f",
    "kzg-point-evaluation",
];

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
    precompile_tracking: bool,
    opcode_tracking: bool,
}

impl PersistExecutionReport {
    pub fn new(
        chain_id: u64,
        report_path: PathBuf,
        precompile_tracking: bool,
        opcode_tracking: bool,
    ) -> Self {
        Self { chain_id, report_path, precompile_tracking, opcode_tracking }
    }

    fn write_header(&self, writer: &mut Writer<File>) -> csv::Result<()> {
        let mut headers = vec![
            "chain_id".to_string(),
            "block_number".to_string(),
            "gas_used".to_string(),
            "tx_count".to_string(),
        ];

        if self.opcode_tracking {
            // To be able to track opcodes cycle count, we have to to attach an inspector to the
            // EVM. This incure a huge performance penalty, so it's not relevant to
            // track anything else than opcodes.

            // Add opcodes headers
            let mut opcode_headers = OPCODE_INFO
                .into_iter()
                .flatten()
                .flat_map(|x| [x.name().to_lowercase(), "count".to_string(), "avg".to_string()])
                .collect();
            headers.append(&mut opcode_headers);
        } else {
            // Add cycle count headers
            headers.push("total_cycles_count".to_string());
            headers.push("deserialize_inputs_cycles_count".to_string());
            headers.push("initialize_witness_db_cycles_count".to_string());
            headers.push("recover_senders_cycles_count".to_string());
            headers.push("block_execution_cycles_count".to_string());
            headers.push("block_validation_cycles_count".to_string());
            headers.push("accrue_logs_bloom_cycles_count".to_string());
            headers.push("state_root_computation_cycles_count".to_string());
            headers.push("syscalls_count".to_string());
            headers.push("prover_gas".to_string());

            // Add syscall headers
            for s in SyscallCode::iter() {
                headers.push(s.to_string().to_lowercase());
            }

            if self.precompile_tracking {
                // Add precompile headers
                let mut precompile_headers = PRECOMPILES
                    .iter()
                    .flat_map(|x| [x.to_string(), "count".to_string(), "avg".to_string()])
                    .collect();
                headers.append(&mut precompile_headers);
            }
        }

        writer.write_record(&headers)
    }

    fn write_record<P: NodePrimitives>(
        &self,
        writer: &mut Writer<File>,
        block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
    ) -> csv::Result<()> {
        let mut record = vec![
            self.chain_id.to_string(),
            block.number.to_string(),
            block.header.gas_used().to_string(),
            block.body.transaction_count().to_string(),
        ];

        if self.opcode_tracking {
            for o in OPCODE_INFO.into_iter().flatten() {
                add_metrics(
                    format!("opcode-{}", o.name().to_lowercase()),
                    &mut record,
                    execution_report,
                );
            }
        } else {
            record.push(execution_report.total_instruction_count().to_string());
            record.push(
                execution_report.cycle_tracker.get(DESERIALZE_INPUTS).unwrap_or(&0).to_string(),
            );
            record.push(
                execution_report.cycle_tracker.get(INIT_WITNESS_DB).unwrap_or(&0).to_string(),
            );
            record.push(
                execution_report.cycle_tracker.get(RECOVER_SENDERS).unwrap_or(&0).to_string(),
            );
            record.push(
                execution_report.cycle_tracker.get(BLOCK_EXECUTION).unwrap_or(&0).to_string(),
            );
            record.push(
                execution_report.cycle_tracker.get(VALIDATE_EXECUTION).unwrap_or(&0).to_string(),
            );
            record.push(
                execution_report.cycle_tracker.get(COMPUTE_STATE_ROOT).unwrap_or(&0).to_string(),
            );
            record.push(execution_report.total_syscall_count().to_string());
            record.push(execution_report.gas.unwrap_or_default().to_string());

            for s in SyscallCode::iter() {
                record.push(execution_report.syscall_counts[s].to_string());
            }

            if self.precompile_tracking {
                for p in PRECOMPILES {
                    add_metrics(format!("precompile-{p}"), &mut record, execution_report);
                }
            }
        }

        writer.write_record(&record)
    }
}

/// Adds metrics for the given precompile on the record.
fn add_metrics(name: String, record: &mut Vec<String>, execution_report: &ExecutionReport) {
    let total = execution_report.cycle_tracker.get(&name).unwrap_or(&0);

    let count = execution_report.invocation_tracker.get(&name).unwrap_or(&0);

    record.push(total.to_string());
    record.push(count.to_string());
    record.push(total.checked_div(*count).unwrap_or(0).to_string());
}

impl ExecutionHooks for PersistExecutionReport {
    async fn on_execution_end<P: NodePrimitives>(
        &self,
        executed_block: &Block<P::SignedTx>,
        execution_report: &ExecutionReport,
    ) -> eyre::Result<()> {
        println!("\nExecution report:\n{}", execution_report);

        // Open the file for appending or create it if it doesn't exist
        let file = OpenOptions::new().append(true).create(true).open(self.report_path.clone())?;

        // Check if the file is empty
        let file_is_empty = file.metadata()?.len() == 0;
        let mut writer = WriterBuilder::new().from_writer(file);

        if file_is_empty {
            self.write_header(&mut writer)?;
        }

        self.write_record::<P>(&mut writer, executed_block, execution_report)?;

        writer.flush()?;

        Ok(())
    }
}
