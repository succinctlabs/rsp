//! Experimental arena-based, zero-copy Merkle Patricia Trie.
//!
//! Ported from the `openvm-eth` fork of RSP. Compared to the legacy pointer-based `MptNode`
//! (`crate::mpt`), this implementation:
//!
//! * stores every node in a flat `Vec`, referencing children by `u32` index (cache-friendly, no
//!   per-node `Box` allocation);
//! * borrows leaf/extension paths and values directly from the input buffer or a `bumpalo` bump
//!   arena (zero-copy), instead of owning heap `Vec`s;
//! * caches node references with `Cell` instead of a per-node `Mutex`;
//! * traverses keys without ever materializing a `Vec<u8>` of nibbles.
//!
//! These properties matter inside a zkVM, where heap allocation is proven cycle-for-cycle.

mod trie;
pub use trie::*;

mod bump_bufmut;
mod hp;
mod node;

#[cfg(test)]
mod tests;
