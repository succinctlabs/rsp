//! Host script: runs the MPT benchmark guest inside the SP1 zkVM executor and reports
//! per-phase **cycle counts** and total **prover gas** for the legacy vs. arena MPT.
//!
//! Modeled on `sp1/examples/rsp/script/src/main.rs`.
//!
//! Usage (requires the SP1 toolchain — <https://docs.succinct.xyz/docs/sp1/getting-started>):
//!
//! ```text
//! cd examples/sp1-mpt-bench/script
//! cargo run --release -- [N]      # N = number of entries, default 2000
//! ```
//!
//! `cargo run` triggers `build.rs`, which compiles the guest program with `sp1-build`.
//! `client.execute(...)` only runs the RISC-V executor (no proving) — fast, no GPU needed —
//! and returns an `ExecutionReport` carrying the cycle tracker and prover-gas figures.

use sp1_sdk::{include_elf, utils, Elf, ProverClient, SP1Stdin};

/// The guest ELF, embedded at compile time by `sp1-build`.
const ELF: Elf = include_elf!("sp1-mpt-bench-program");

#[tokio::main]
async fn main() {
    utils::setup_logger();

    let n: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
    println!("SP1 MPT benchmark — N = {n} keccak-keyed entries\n");

    let mut stdin = SP1Stdin::new();
    stdin.write(&n);

    let client = ProverClient::from_env().await;
    let now = std::time::Instant::now();
    let (_public_values, report) = client.execute(ELF, stdin).await.unwrap();
    println!("executor wall-time: {:?}\n", now.elapsed());

    let cycles = |name: &str| report.cycle_tracker.get(name).copied().unwrap_or(0);
    let ratio = |legacy: u64, arena: u64| {
        if arena == 0 {
            f64::INFINITY
        } else {
            legacy as f64 / arena as f64
        }
    };

    println!("{:<8} {:>16} {:>16} {:>14}", "phase", "legacy cycles", "arena cycles", "legacy/arena");
    println!("{}", "-".repeat(58));
    for (label, legacy_key, arena_key) in [
        ("build", "legacy-build", "arena-build"),
        ("get", "legacy-get", "arena-get"),
        ("delete", "legacy-delete", "arena-delete"),
    ] {
        let l = cycles(legacy_key);
        let a = cycles(arena_key);
        println!("{label:<8} {l:>16} {a:>16} {:>13.2}x", ratio(l, a));
    }

    println!("\narena-only phases (witness serialization round-trip):");
    println!("  encode : {:>16} cycles", cycles("arena-encode"));
    println!("  decode : {:>16} cycles", cycles("arena-decode"));

    println!("\nwhole-program totals:");
    println!("  cycles   : {}", report.total_instruction_count());
    println!("  syscalls : {}", report.total_syscall_count());
    match report.gas() {
        Some(gas) => println!("  prover gas: {gas}"),
        None => println!("  prover gas: (not reported by this sp1-sdk build)"),
    }
}
