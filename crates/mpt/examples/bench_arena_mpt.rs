//! Benchmark: legacy pointer-based `MptNode` vs. the arena-based `arena::Mpt`.
//!
//! Run with:
//!
//! ```text
//! cargo run --release --example bench_arena_mpt -p rsp-mpt
//! ```
//!
//! There is no SP1 toolchain in scope here, so this cannot report true zkVM cycle counts.
//! Instead it reports two host-side proxies:
//!
//! * wall-clock time (min of several runs), and
//! * heap allocation count / bytes via a counting global allocator.
//!
//! Neither proxy is a substitute for a real SP1 cycle count, and they can even disagree with it:
//! native wall-time benefits from SIMD/`memcpy` that the RISC-V zkVM does not have, while the
//! allocation count's weight depends on SP1's guest allocator. Treat the allocation *count* as
//! the structural signal (the arena design removes per-node allocation entirely) and the
//! definitive comparison as a follow-up `cargo prove` cycle measurement.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicU64, Ordering::Relaxed},
    time::Instant,
};

use alloy_primitives::{keccak256, B256};
use rsp_mpt::{arena::Mpt, MptNode};

// ---------------------------------------------------------------------------
// Counting global allocator
// ---------------------------------------------------------------------------

struct CountingAlloc;

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Relaxed);
        BYTES_ALLOCATED.fetch_add(layout.size() as u64, Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Relaxed);
        if new_size > layout.size() {
            BYTES_ALLOCATED.fetch_add((new_size - layout.size()) as u64, Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[derive(Clone, Copy)]
struct Stats {
    allocs: u64,
    bytes: u64,
}

fn snapshot() -> Stats {
    Stats { allocs: ALLOC_COUNT.load(Relaxed), bytes: BYTES_ALLOCATED.load(Relaxed) }
}

/// Runs `f`, returning `(result, allocations during f, bytes allocated during f)`.
fn measure_allocs<R>(f: impl FnOnce() -> R) -> (R, Stats) {
    let before = snapshot();
    let r = f();
    let after = snapshot();
    (r, Stats { allocs: after.allocs - before.allocs, bytes: after.bytes - before.bytes })
}

/// Returns the minimum wall-clock duration of `iters` runs of `f`, in microseconds.
fn min_time_us(iters: u32, mut f: impl FnMut()) -> u128 {
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        f();
        best = best.min(t.elapsed().as_micros());
    }
    best
}

// ---------------------------------------------------------------------------
// Workload
// ---------------------------------------------------------------------------

const N: usize = 5_000;
const TIME_ITERS: u32 = 7;

/// Deterministic, keccak-hashed 32-byte keys, like hashed account addresses / storage slots.
fn keys() -> Vec<B256> {
    (0..N as u64).map(|i| keccak256(i.to_be_bytes())).collect()
}

fn main() {
    let keys = keys();

    println!("Arena MPT benchmark  (N = {N} keccak-keyed entries)\n");

    // --- correctness cross-check: both implementations must agree on the root hash ---
    let legacy_root = {
        let mut t = MptNode::default();
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        t.hash()
    };
    let arena_root = {
        let bump = bumpalo::Bump::new();
        let mut t = Mpt::new(&bump);
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        t.hash()
    };
    assert_eq!(legacy_root, arena_root, "root hash mismatch between implementations");
    println!("root hashes agree: {legacy_root}\n");

    // --- BUILD (insert N + compute root hash) ---
    let (_, legacy_build) = measure_allocs(|| {
        let mut t = MptNode::default();
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        t.hash()
    });
    let (_, arena_build) = measure_allocs(|| {
        let bump = bumpalo::Bump::new();
        let mut t = Mpt::new(&bump);
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        t.hash()
    });
    let legacy_build_t = min_time_us(TIME_ITERS, || {
        let mut t = MptNode::default();
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        std::hint::black_box(t.hash());
    });
    let arena_build_t = min_time_us(TIME_ITERS, || {
        let bump = bumpalo::Bump::new();
        let mut t = Mpt::new(&bump);
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        std::hint::black_box(t.hash());
    });

    // --- GET (look up all N keys) ---
    let legacy_get_t;
    let arena_get_t;
    let legacy_get;
    let arena_get;
    {
        let mut legacy = MptNode::default();
        for (i, k) in keys.iter().enumerate() {
            legacy.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        (_, legacy_get) = measure_allocs(|| {
            for k in &keys {
                std::hint::black_box(legacy.get_rlp::<u64>(k.as_slice()).unwrap());
            }
        });
        legacy_get_t = min_time_us(TIME_ITERS, || {
            for k in &keys {
                std::hint::black_box(legacy.get_rlp::<u64>(k.as_slice()).unwrap());
            }
        });

        let bump = bumpalo::Bump::new();
        let mut arena = Mpt::new(&bump);
        for (i, k) in keys.iter().enumerate() {
            arena.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        (_, arena_get) = measure_allocs(|| {
            for k in &keys {
                std::hint::black_box(arena.get_rlp::<u64>(k.as_slice()).unwrap());
            }
        });
        arena_get_t = min_time_us(TIME_ITERS, || {
            for k in &keys {
                std::hint::black_box(arena.get_rlp::<u64>(k.as_slice()).unwrap());
            }
        });
    }

    // --- DELETE (remove all N keys) ---
    // The trie is built first (outside the measured region), then deletion is measured alone.
    let legacy_del = {
        let mut t = MptNode::default();
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        let (_, s) = measure_allocs(|| {
            for k in &keys {
                t.delete(k.as_slice()).unwrap();
            }
        });
        s
    };
    let arena_del = {
        let bump = bumpalo::Bump::new();
        let mut t = Mpt::new(&bump);
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        let (_, s) = measure_allocs(|| {
            for k in &keys {
                t.delete(k.as_slice()).unwrap();
            }
        });
        s
    };

    // --- ENCODE / DECODE (witness serialization round-trip) ---
    let arena_encdec = {
        let bump = bumpalo::Bump::new();
        let mut t = Mpt::new(&bump);
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        let num_nodes = t.num_nodes();
        let encoded = t.encode_trie();
        let bump2 = bumpalo::Bump::new();
        let (_, s) = measure_allocs(|| {
            let mut slice = encoded.as_slice();
            let decoded = Mpt::decode_trie(&bump2, &mut slice, num_nodes).unwrap();
            std::hint::black_box(decoded.hash());
        });
        (encoded.len(), s)
    };
    // The legacy trie is serialized the way RSP actually ships it in the witness: serde over
    // the whole node tree (here via bincode). NOTE: `alloy_rlp::encode` would NOT work as a
    // whole-trie encoding -- it emits only the root node with 32-byte digest child references.
    let legacy_encdec = {
        let mut t = MptNode::default();
        for (i, k) in keys.iter().enumerate() {
            t.insert_rlp(k.as_slice(), i as u64).unwrap();
        }
        let encoded = bincode::serialize(&t).unwrap();
        let (_, s) = measure_allocs(|| {
            let decoded: MptNode = bincode::deserialize(&encoded).unwrap();
            std::hint::black_box(decoded.hash());
        });
        (encoded.len(), s)
    };

    // --- report ---
    print_phase(
        "BUILD  (insert N + hash)",
        legacy_build,
        legacy_build_t,
        arena_build,
        arena_build_t,
    );
    print_phase("GET    (lookup N)", legacy_get, legacy_get_t, arena_get, arena_get_t);
    print_phase_no_time("DELETE (remove N)", legacy_del, arena_del);

    println!("\nDECODE (witness deserialization round-trip)");
    println!(
        "  legacy : encoded {:>7} B   {:>9} allocs   {:>10} B allocated",
        legacy_encdec.0, legacy_encdec.1.allocs, legacy_encdec.1.bytes
    );
    println!(
        "  arena  : encoded {:>7} B   {:>9} allocs   {:>10} B allocated",
        arena_encdec.0, arena_encdec.1.allocs, arena_encdec.1.bytes
    );
}

fn print_phase(name: &str, l: Stats, lt: u128, a: Stats, at: u128) {
    println!("{name}");
    println!("  legacy : {:>9} allocs   {:>11} B allocated   {:>8} us", l.allocs, l.bytes, lt);
    println!("  arena  : {:>9} allocs   {:>11} B allocated   {:>8} us", a.allocs, a.bytes, at);
    println!(
        "  ratio  : {:>8.1}x fewer allocs   {:>8.1}x fewer bytes   {:>7.1}x faster\n",
        ratio(l.allocs, a.allocs),
        ratio(l.bytes, a.bytes),
        ratio(lt as u64, at as u64),
    );
}

fn print_phase_no_time(name: &str, l: Stats, a: Stats) {
    println!("{name}");
    println!("  legacy : {:>9} allocs   {:>11} B allocated", l.allocs, l.bytes);
    println!("  arena  : {:>9} allocs   {:>11} B allocated", a.allocs, a.bytes);
    println!(
        "  ratio  : {:>8.1}x fewer allocs   {:>8.1}x fewer bytes\n",
        ratio(l.allocs, a.allocs),
        ratio(l.bytes, a.bytes),
    );
}

fn ratio(legacy: u64, arena: u64) -> f64 {
    if arena == 0 {
        f64::INFINITY
    } else {
        legacy as f64 / arena as f64
    }
}
