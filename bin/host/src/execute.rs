use csv::Writer;
use rsp_client_executor::{io::ClientExecutorInput, ChainVariant};
use sp1_sdk::ExecutionReport;
use std::{fs::OpenOptions, path::PathBuf};

/// Given an execution report, print it out and write it to a CSV specified by report_path.
pub fn process_execution_report(
    variant: ChainVariant,
    client_input: ClientExecutorInput,
    execution_report: ExecutionReport,
    report_path: PathBuf,
) -> eyre::Result<()> {
    println!("\nExecution report:\n{}", execution_report);

    let chain_id = variant.chain_id();
    let executed_block = client_input.current_block;
    let block_number = executed_block.header.number;
    let gas_used = executed_block.header.gas_used;
    let tx_count = executed_block.body.len();
    let number_cycles = execution_report.total_instruction_count();
    let number_syscalls = execution_report.total_syscall_count();

    let bn_add_cycles = execution_report.cycle_tracker.get("precompile-bn-add").unwrap_or(&0);
    let bn_mul_cycles = execution_report.cycle_tracker.get("precompile-bn-mul").unwrap_or(&0);
    let bn_pair_cycles = execution_report.cycle_tracker.get("precompile-bn-pair").unwrap_or(&0);
    let kzg_cyles =
        execution_report.cycle_tracker.get("precompile-kzg-point-evaluation").unwrap_or(&0);

    // TODO: we can track individual syscalls in our CSV once we have sp1-core as a dependency
    // let keccak_count = execution_report.syscall_counts.get(SyscallCode::KECCAK_PERMUTE);
    // let secp256k1_decompress_count =
    //     execution_report.syscall_counts.get(SyscallCode::SECP256K1_DECOMPRESS);

    let report_file_exists = report_path.exists();

    // Open the file for appending or create it if it doesn't exist
    let file = OpenOptions::new().append(true).create(true).open(report_path)?;

    let mut writer = Writer::from_writer(file);

    // Write the header if the file doesn't exist
    if !report_file_exists {
        writer.write_record([
            "chain_id",
            "block_number",
            "gas_used",
            "tx_count",
            "number_cycles",
            "number_syscalls",
            "bn_add_cycles",
            "bn_mul_cycles",
            "bn_pair_cycles",
            "kzg_point_eval_cycles",
        ])?;
    }

    // Write the data
    writer.write_record(&[
        chain_id.to_string(),
        block_number.to_string(),
        gas_used.to_string(),
        tx_count.to_string(),
        number_cycles.to_string(),
        number_syscalls.to_string(),
        bn_add_cycles.to_string(),
        bn_mul_cycles.to_string(),
        bn_pair_cycles.to_string(),
        kzg_cyles.to_string(),
    ])?;

    writer.flush()?;

    Ok(())
}
