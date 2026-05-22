//! SP1 guest program: measures zkVM cycle counts for the legacy pointer-based MPT
//! (`rsp_mpt::MptNode`) vs. the experimental arena-based MPT (`rsp_mpt::arena::Mpt`).
//!
//! The guest runs a single configuration per invocation, selected by `mode`, so the host can
//! compare total cycle counts across runs (the per-phase `cycle-tracker` report is unreliable
//! in this standalone setup). `mode`: 0 = baseline (key generation only), 1 = legacy MPT,
//! 2 = arena MPT. Subtracting the baseline isolates the MPT cost.

#![no_main]
sp1_zkvm::entrypoint!(main);

use alloy_primitives::{keccak256, B256};
use rsp_mpt::{arena::Mpt, MptNode};

pub fn main() {
    // Number of entries and the configuration to run; written by the host script.
    let n = sp1_zkvm::io::read::<u32>() as usize;
    let mode = sp1_zkvm::io::read::<u32>();

    // Keccak-hashed keys: uniformly distributed, like hashed account addresses / storage slots.
    // Generated identically in every mode so the baseline cancels key-derivation cost out.
    let keys: Vec<[u8; 32]> = (0..n as u64).map(|i| keccak256(i.to_be_bytes()).0).collect();

    let root: B256 = match mode {
        // Baseline: only the (shared) key generation, so its cost can be subtracted out.
        0 => {
            core::hint::black_box(&keys);
            B256::from(keys[n - 1])
        }
        // Legacy pointer-based MPT: build + get + delete.
        1 => {
            let mut trie = MptNode::default();
            for (i, k) in keys.iter().enumerate() {
                trie.insert_rlp(k.as_slice(), i as u64).unwrap();
            }
            let root = trie.hash();
            for k in &keys {
                core::hint::black_box(trie.get_rlp::<u64>(k.as_slice()).unwrap());
            }
            for k in &keys {
                trie.delete(k.as_slice()).unwrap();
            }
            root
        }
        // Arena-based MPT: build + get + delete.
        _ => {
            let bump = bumpalo::Bump::new();
            let mut trie = Mpt::new(&bump);
            for (i, k) in keys.iter().enumerate() {
                trie.insert_rlp(k.as_slice(), i as u64).unwrap();
            }
            let root = trie.hash();
            for k in &keys {
                core::hint::black_box(trie.get_rlp::<u64>(k.as_slice()).unwrap());
            }
            for k in &keys {
                trie.delete(k.as_slice()).unwrap();
            }
            root
        }
    };

    sp1_zkvm::io::commit_slice(root.as_slice());
}
