//! Host script: runs the MPT benchmark guest inside the SP1 zkVM executor and compares total
//! **cycle counts** for the legacy vs. arena MPT.
//!
//! The guest runs one configuration per invocation (mode 0 = baseline/keygen, 1 = legacy,
//! 2 = arena); subtracting the baseline isolates the MPT cost. `client.execute(...)` only runs
//! the RISC-V executor (no proving) — fast, no GPU needed.
//!
//! Usage:
//! ```text
//! cd examples/sp1-mpt-bench/script
//! cargo run --release -- [N]      # N = number of entries, default 2000
//! ```

use sp1_sdk::{include_elf, utils, Elf, Prover, ProverClient, SP1Stdin};

/// The guest ELF, embedded at compile time by `sp1-build`.
const ELF: Elf = include_elf!("sp1-mpt-bench-program");

#[tokio::main]
async fn main() {
    utils::setup_logger();

    let n: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
    println!("SP1 MPT benchmark — N = {n} keccak-keyed entries\n");

    let client = ProverClient::from_env().await;

    let mut totals = [0u64; 3];
    let mut syscalls = [0u64; 3];
    let mut gas = [0u64; 3];
    let mut roots: [Vec<u8>; 3] = Default::default();

    for mode in 0..3u32 {
        let mut stdin = SP1Stdin::new();
        stdin.write(&n);
        stdin.write(&mode);
        let (public_values, report) = client.execute(ELF, stdin).await.unwrap();
        totals[mode as usize] = report.total_instruction_count();
        syscalls[mode as usize] = report.total_syscall_count();
        gas[mode as usize] = report.gas().unwrap_or(0);
        roots[mode as usize] = public_values.as_slice().to_vec();
    }

    // Correctness: legacy and arena must agree on the root hash.
    assert_eq!(roots[1], roots[2], "legacy/arena root mismatch");

    let legacy = totals[1].saturating_sub(totals[0]);
    let arena = totals[2].saturating_sub(totals[0]);

    println!("{:<26} {:>16} {:>16}", "metric", "legacy", "arena");
    println!("{}", "-".repeat(60));
    println!("{:<26} {:>16} {:>16}", "total cycles (raw)", totals[1], totals[2]);
    println!("{:<26} {:>16} {:>16}", "MPT cycles (- baseline)", legacy, arena);
    println!("{:<26} {:>16} {:>16}", "syscalls", syscalls[1], syscalls[2]);
    println!("{:<26} {:>16} {:>16}", "prover gas", gas[1], gas[2]);
    println!("\nbaseline (keygen only) cycles: {}", totals[0]);
    if arena > 0 {
        println!("MPT cycle ratio legacy/arena : {:.3}x", legacy as f64 / arena as f64);
    }
}
