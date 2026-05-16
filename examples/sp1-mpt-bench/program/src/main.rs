//! SP1 guest program: measures zkVM cycle counts for the legacy pointer-based MPT
//! (`rsp_mpt::MptNode`) vs. the experimental arena-based MPT (`rsp_mpt::arena::Mpt`).
//!
//! Each phase is wrapped in `cycle-tracker-report-{start,end}` markers so the host script
//! can read per-phase cycle counts out of the `ExecutionReport`.

#![no_main]
sp1_zkvm::entrypoint!(main);

use alloy_primitives::keccak256;
use rsp_mpt::{arena::Mpt, MptNode};

pub fn main() {
    // Number of entries to insert; written by the host script.
    let n = sp1_zkvm::io::read::<u32>() as usize;

    // Keccak-hashed keys: uniformly distributed, like hashed account addresses / storage slots.
    // Generated outside the measured regions so key derivation does not skew the comparison.
    let keys: Vec<[u8; 32]> = (0..n as u64).map(|i| keccak256(i.to_be_bytes()).0).collect();

    // ----------------------------- legacy pointer-based MPT -----------------------------
    let mut legacy = MptNode::default();

    println!("cycle-tracker-report-start: legacy-build");
    for (i, k) in keys.iter().enumerate() {
        legacy.insert_rlp(k.as_slice(), i as u64).unwrap();
    }
    let legacy_root = legacy.hash();
    println!("cycle-tracker-report-end: legacy-build");

    println!("cycle-tracker-report-start: legacy-get");
    for k in &keys {
        core::hint::black_box(legacy.get_rlp::<u64>(k.as_slice()).unwrap());
    }
    println!("cycle-tracker-report-end: legacy-get");

    println!("cycle-tracker-report-start: legacy-delete");
    for k in &keys {
        legacy.delete(k.as_slice()).unwrap();
    }
    println!("cycle-tracker-report-end: legacy-delete");

    // ------------------------------- arena-based MPT ------------------------------------
    let bump = bumpalo::Bump::new();
    let mut arena = Mpt::new(&bump);

    println!("cycle-tracker-report-start: arena-build");
    for (i, k) in keys.iter().enumerate() {
        arena.insert_rlp(k.as_slice(), i as u64).unwrap();
    }
    let arena_root = arena.hash();
    println!("cycle-tracker-report-end: arena-build");

    println!("cycle-tracker-report-start: arena-get");
    for k in &keys {
        core::hint::black_box(arena.get_rlp::<u64>(k.as_slice()).unwrap());
    }
    println!("cycle-tracker-report-end: arena-get");

    // Witness serialization round-trip (arena format only; see notes in the host script).
    println!("cycle-tracker-report-start: arena-encode");
    let encoded = arena.encode_trie();
    let num_nodes = arena.num_nodes();
    println!("cycle-tracker-report-end: arena-encode");

    let decode_bump = bumpalo::Bump::new();
    println!("cycle-tracker-report-start: arena-decode");
    let mut slice = encoded.as_slice();
    let decoded = Mpt::decode_trie(&decode_bump, &mut slice, num_nodes).unwrap();
    let decoded_root = decoded.hash();
    println!("cycle-tracker-report-end: arena-decode");
    assert_eq!(decoded_root, arena_root, "decoded root must match");

    println!("cycle-tracker-report-start: arena-delete");
    for k in &keys {
        arena.delete(k.as_slice()).unwrap();
    }
    println!("cycle-tracker-report-end: arena-delete");

    // Correctness: both implementations must agree on the root hash.
    assert_eq!(legacy_root, arena_root, "legacy/arena root mismatch");

    // Commit the (shared) root hash as the public output.
    sp1_zkvm::io::commit_slice(legacy_root.as_slice());
}
