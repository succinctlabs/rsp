#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_primitives::{keccak256, map::HashMap, Address, B256, U256};
use alloy_rlp::Decodable;
use alloy_rpc_types::EIP1186AccountProofResponse;
use reth_trie::{AccountProof, HashedPostState, TrieAccount};
use serde::{Deserialize, Serialize};

#[cfg(feature = "execution-witness")]
mod execution_witness;

/// Module containing MPT code adapted from `zeth`.
mod mpt;
pub use mpt::Error;

/// Arena-based, zero-copy MPT used as the witness format on the host->guest boundary
/// (ported from `openvm-eth`).
pub mod arena;

use mpt::{
    mpt_from_proof, parse_proof, proofs_to_tries, resolve_nodes, transition_proofs_to_tries,
};

/// Legacy pointer-based MPT node — used host-side to build the witness from RPC proofs before
/// re-encoding into the arena codec. Not used in the guest (which only sees the arena form).
pub use mpt::MptNode;

/// Ethereum state trie and account storage tries.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EthereumState {
    pub state_trie: MptNode,
    pub storage_tries: HashMap<B256, MptNode>,
}

impl EthereumState {
    /// Builds Ethereum state tries from relevant proofs before and after a state transition.
    pub fn from_transition_proofs(
        state_root: B256,
        parent_proofs: &HashMap<Address, AccountProof>,
        proofs: &HashMap<Address, AccountProof>,
    ) -> Result<Self, FromProofError> {
        transition_proofs_to_tries(state_root, parent_proofs, proofs)
    }

    /// Builds Ethereum state tries from relevant proofs from a given state.
    pub fn from_proofs(
        state_root: B256,
        proofs: &HashMap<Address, AccountProof>,
    ) -> Result<Self, FromProofError> {
        proofs_to_tries(state_root, proofs)
    }

    /// Builds Ethereum state tries from a EIP-1186 proof.
    pub fn from_account_proof(proof: EIP1186AccountProofResponse) -> Result<Self, FromProofError> {
        let mut storage_tries = HashMap::with_hasher(Default::default());
        let mut storage_nodes = HashMap::with_hasher(Default::default());
        let mut storage_root_node = MptNode::default();

        for storage_proof in &proof.storage_proof {
            let proof_nodes = parse_proof(&storage_proof.proof)?;
            mpt_from_proof(&proof_nodes)?;

            // the first node in the proof is the root
            if let Some(node) = proof_nodes.first() {
                storage_root_node = node.clone();
            }

            proof_nodes.into_iter().for_each(|node| {
                storage_nodes.insert(node.reference(), node);
            });
        }

        storage_tries
            .insert(keccak256(proof.address), resolve_nodes(&storage_root_node, &storage_nodes));

        let state = EthereumState {
            state_trie: MptNode::from_account_proof(&proof.account_proof)?,
            storage_tries,
        };

        Ok(state)
    }

    #[cfg(feature = "execution-witness")]
    pub fn from_execution_witness(
        witness: &alloy_rpc_types_debug::ExecutionWitness,
        pre_state_root: B256,
    ) -> Self {
        let (state_trie, storage_tries) =
            execution_witness::build_validated_tries(witness, pre_state_root).unwrap();

        Self { state_trie, storage_tries }
    }

    /// Mutates state based on diffs provided in [`HashedPostState`].
    ///
    /// Applies changes in the canonical-witness-spec order:
    ///   - keys sorted lexicographically ascending,
    ///   - inserts/updates applied before deletions,
    ///   - same convention at both storage-trie and state-trie levels.
    ///
    /// This is the order the `Canonical` `ExecutionWitnessMode` assumes when emitting siblings
    /// — see `ethereum/execution-specs@projects/zkevm`. Legacy mode tolerates any order (it
    /// includes a strict superset of canonical's siblings), so this single implementation
    /// works for both witness shapes.
    ///
    /// For accounts whose state changed but storage was *not* touched in this block, canonical
    /// omits the storage trie itself; we read the pre-existing `storage_root` from the account
    /// leaf in the state trie rather than calling `hash()` on a freshly-defaulted empty trie.
    pub fn update(&mut self, post_state: &HashedPostState) {
        type AcctRef<'a> = (
            &'a B256,
            Option<&'a reth_primitives_traits::Account>,
            Option<&'a reth_trie::HashedStorage>,
        );

        // Sort accounts once, pre-pairing each with its (account-change, storage-change) so
        // the two passes don't re-look-up the HashedPostState HashMaps per element.
        let mut accounts_sorted: Vec<AcctRef<'_>> = post_state
            .accounts
            .iter()
            .map(|(k, v)| (k, v.as_ref(), post_state.storages.get(k)))
            .collect();
        accounts_sorted.sort_unstable_by(|a, b| a.0.cmp(b.0));

        // Reusable scratch buffers for per-account storage-slot sorting. Sized for typical
        // contract touch counts (~1-32 slots/block); grows if needed, never shrinks.
        let mut slot_inserts: Vec<(B256, U256)> = Vec::with_capacity(32);
        let mut slot_deletes: Vec<B256> = Vec::with_capacity(32);

        // -- Pass 1: storage tries (per Some-account) + state-trie inserts/updates --
        for entry in &accounts_sorted {
            let hashed_address: &B256 = entry.0;
            let Some(account) = entry.1 else { continue };
            let storage_change: Option<&reth_trie::HashedStorage> = entry.2;

            let storage_root = if let Some(ss) = storage_change {
                // Storage touched in this block — apply two-pass canonical update.
                let storage_trie = self.storage_tries.entry(*hashed_address).or_default();
                if ss.wiped {
                    storage_trie.clear();
                }

                slot_inserts.clear();
                slot_deletes.clear();
                for (k, v) in ss.storage.iter() {
                    if v.is_zero() {
                        slot_deletes.push(*k);
                    } else {
                        slot_inserts.push((*k, *v));
                    }
                }
                slot_inserts.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                slot_deletes.sort_unstable();

                for (k, v) in &slot_inserts {
                    storage_trie.insert_rlp(k.as_slice(), *v).unwrap();
                }
                for k in &slot_deletes {
                    storage_trie.delete(k.as_slice()).unwrap();
                }

                storage_trie.hash()
            } else {
                // Storage NOT touched — preserve the pre-existing storage_root. Canonical mode
                // may have omitted the storage trie entirely, so we can't `hash()` an empty
                // entry; instead read the account's prior storage_root from the existing leaf.
                self.state_trie
                    .get_rlp::<TrieAccount>(hashed_address.as_slice())
                    .ok()
                    .flatten()
                    .map(|a| a.storage_root)
                    .unwrap_or(reth_trie::EMPTY_ROOT_HASH)
            };

            let state_account = TrieAccount {
                nonce: account.nonce,
                balance: account.balance,
                storage_root,
                code_hash: account.get_bytecode_hash(),
            };
            self.state_trie.insert_rlp(hashed_address.as_slice(), state_account).unwrap();
        }

        // -- Pass 2: state-trie deletes (sorted) --
        for entry in &accounts_sorted {
            if entry.1.is_none() {
                self.state_trie.delete(entry.0.as_slice()).unwrap();
            }
        }
    }

    /// Computes the state root.
    pub fn state_root(&self) -> B256 {
        self.state_trie.hash()
    }

    /// Encodes the state into a single flat byte blob in the arena codec — the zero-copy
    /// witness. All tries are `encode_trie`d and concatenated with framing
    /// (num_nodes/length/key), and the guest reconstructs them zero-copy by borrowing the blob
    /// directly with [`ArenaTries::decode`].
    ///
    /// Wire layout:
    /// ```text
    /// [state_trie: num_nodes:u32 | len:u32 | bytes:len]
    /// [num_storage_tries:u32]
    /// [storage_tries: per entry { addr:32 | num_nodes:u32 | len:u32 | bytes:len }]
    /// [num_psr:u32]
    /// [psr: per entry { hashed_address:32 | storage_root:32 } sorted by hashed_address]
    /// ```
    ///
    /// The trailing `psr` section is the **pre-state storage_root cache**: for every revealed
    /// account in `state_trie`, the `storage_root` field of its account leaf. Sorted by
    /// hashed_address so the guest's `ArenaTries::update` coordinated walk is O(N+M).
    ///
    /// Why we ship this: canonical witness mode omits storage tries for accounts whose state
    /// changes but whose storage isn't touched in the block. Without the cache, the guest has
    /// to walk the state trie to recover each affected account's pre-existing storage_root —
    /// O(depth) per fallback. With the cache, it's a single linear scan across the whole
    /// update. See `ArenaTries::update` for the consumer side.
    ///
    /// SOUNDNESS: the psr section is **not separately hash-committed**. The host can put any
    /// value (or omit entries, or include spurious ones). The cache is sound because:
    ///
    ///   1. Every cached `storage_root` flows into a `state_account.storage_root` in
    ///      `ArenaTries::update`'s no-storage-change fallback, then into the state-trie leaf,
    ///      then into the final state-trie root via the hashes-of-children chain.
    ///   2. The guest's `ClientExecutor::execute` compares the final state root against
    ///      `input.current_block.header().state_root()` and returns `MismatchedStateRoot` on
    ///      any disagreement (see `crates/executor/client/src/executor.rs:127` — flagged with
    ///      a `SOUNDNESS:` tag).
    ///   3. The host can't forge an `input.current_block.header().state_root()` matching a
    ///      tampered psr because the committed header is checked by the external verifier
    ///      against the canonical chain.
    ///
    /// Any divergence in psr therefore surfaces as a guest-side error and no proof is produced.
    /// The integration test `psr_tamper_changes_state_root` in this module is the executable
    /// regression check for this property.
    pub fn to_arena_witness(&self) -> Vec<u8> {
        fn put_u32(blob: &mut Vec<u8>, v: usize) {
            blob.extend_from_slice(&(v as u32).to_le_bytes());
        }
        fn put_trie(blob: &mut Vec<u8>, node: &MptNode) {
            let bump = bumpalo::Bump::new();
            let trie = arena::Mpt::from_mpt_node(&bump, node);
            let bytes = trie.encode_trie();
            put_u32(blob, trie.num_nodes());
            put_u32(blob, bytes.len());
            blob.extend_from_slice(&bytes);
        }

        let mut blob = Vec::new();
        put_trie(&mut blob, &self.state_trie);
        put_u32(&mut blob, self.storage_tries.len());
        for (key, node) in &self.storage_tries {
            blob.extend_from_slice(key.as_slice());
            put_trie(&mut blob, node);
        }

        // Pre-state storage_root cache: walk every revealed account leaf, RLP-decode it to
        // pull the `storage_root` field, push `(hashed_address, storage_root)` into a Vec, sort
        // lex by hashed_address. The walk + decode is host CPU only (not proven), so its cost
        // is irrelevant — the gain is in the guest where this section replaces ~250
        // `state_trie.get_rlp::<TrieAccount>` calls per block.
        let mut psr: Vec<(B256, B256)> = Vec::new();
        self.state_trie.for_each_leaves(|key, mut value| {
            let account = TrieAccount::decode(&mut value).unwrap();
            psr.push((B256::from_slice(key), account.storage_root));
        });
        psr.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        put_u32(&mut blob, psr.len());
        for (hashed_address, storage_root) in &psr {
            blob.extend_from_slice(hashed_address.as_slice());
            blob.extend_from_slice(storage_root.as_slice());
        }
        blob
    }
}

/// Decoded arena tries — the guest-side equivalent of [`EthereumState`]. Borrows the bump and
/// the witness blob for the duration of block execution; provides account/storage lookups,
/// `update`, and `state_root`.
#[derive(Debug)]
pub struct ArenaTries<'a> {
    bump: &'a bumpalo::Bump,
    pub state_trie: arena::Mpt<'a>,
    pub storage_tries: HashMap<B256, arena::Mpt<'a>>,
    /// Pre-state `storage_root` per revealed account, sorted by `hashed_address`. Stored
    /// zero-copy as a packed slice (`[hashed_address:32 | storage_root:32]*`). Consumed by
    /// `update` to skip the per-account state-trie walk on the canonical-mode "state changed
    /// but storage untouched" path. See [`EthereumState::to_arena_witness`] for the producer.
    ///
    /// SOUNDNESS: this slice is **untrusted host input** — there is no separate Merkle
    /// commitment over its contents. Soundness derives entirely from the final state-root
    /// check in `ClientExecutor::execute`: any tampered value propagates into the
    /// `state_account.storage_root` of a state-trie leaf, then into the trie root, and the
    /// resulting mismatch surfaces as `MismatchedStateRoot`. See the `SOUNDNESS:` tag in
    /// `crates/executor/client/src/executor.rs` and the regression test
    /// `psr_tamper_changes_state_root` in this crate's tests.
    pre_state_storage_roots: &'a [u8],
}

/// Wire-format constant: bytes per `(hashed_address, storage_root)` entry in the psr section.
const PSR_ENTRY: usize = 64;

impl<'a> ArenaTries<'a> {
    /// Decodes the witness blob into arena tries that borrow `bump` and `blob` directly — no
    /// copy of the witness, one bump allocation, each node's hash verified inline by
    /// [`arena::Mpt::decode_trie`].
    pub fn decode(bump: &'a bumpalo::Bump, blob: &'a [u8]) -> Result<Self, arena::Error> {
        fn read_u32(blob: &[u8], pos: &mut usize) -> usize {
            let v = u32::from_le_bytes(blob[*pos..*pos + 4].try_into().unwrap()) as usize;
            *pos += 4;
            v
        }

        let mut pos = 0usize;
        let decode_trie = |pos: &mut usize| -> Result<arena::Mpt<'a>, arena::Error> {
            let num_nodes = read_u32(blob, pos);
            let len = read_u32(blob, pos);
            let mut bytes: &'a [u8] = &blob[*pos..*pos + len];
            *pos += len;
            arena::Mpt::decode_trie(bump, &mut bytes, num_nodes)
        };

        let state_trie = decode_trie(&mut pos)?;
        let count = read_u32(blob, &mut pos);
        let mut storage_tries = HashMap::with_hasher(Default::default());
        for _ in 0..count {
            let key = B256::from_slice(&blob[pos..pos + 32]);
            pos += 32;
            let trie = decode_trie(&mut pos)?;
            storage_tries.insert(key, trie);
        }

        // Pre-state storage_root cache: zero-copy slice into the blob, sorted by hashed_address.
        let psr_count = read_u32(blob, &mut pos);
        let psr_end = pos + psr_count * PSR_ENTRY;
        let pre_state_storage_roots: &'a [u8] = &blob[pos..psr_end];

        Ok(ArenaTries { bump, state_trie, storage_tries, pre_state_storage_roots })
    }
}

impl ArenaTries<'_> {
    /// Mutates state based on diffs provided in [`HashedPostState`] (mirrors
    /// [`EthereumState::update`] on the arena tries).
    ///
    /// Same canonical ordering as `EthereumState::update`: lexicographic sort + inserts before
    /// deletes, at both storage and state-trie levels. Works on both witness modes.
    /// Falls back to reading the pre-existing storage_root from the account leaf when the
    /// witness omits the storage trie (canonical mode for storage-untouched accounts).
    pub fn update(&mut self, post_state: &HashedPostState) {
        type AcctRef<'a> = (
            &'a B256,
            Option<&'a reth_primitives_traits::Account>,
            Option<&'a reth_trie::HashedStorage>,
        );

        let mut accounts_sorted: Vec<AcctRef<'_>> = post_state
            .accounts
            .iter()
            .map(|(k, v)| (k, v.as_ref(), post_state.storages.get(k)))
            .collect();
        accounts_sorted.sort_unstable_by(|a, b| a.0.cmp(b.0));

        let mut slot_inserts: Vec<(B256, U256)> = Vec::with_capacity(32);
        let mut slot_deletes: Vec<B256> = Vec::with_capacity(32);

        // Coordinated walk through pre_state_storage_roots: both lists are sorted by
        // hashed_address, so a single monotonic cursor across the entire update() makes the
        // fallback lookup O(1) amortized instead of O(trie_depth) per missing-storage-change
        // account.
        let psr = self.pre_state_storage_roots;
        let psr_count = psr.len() / PSR_ENTRY;
        let mut psr_idx: usize = 0;

        // Pass 1: storage tries + state-trie inserts/updates (sorted).
        for entry in &accounts_sorted {
            let hashed_address: &B256 = entry.0;
            let Some(account) = entry.1 else { continue };
            let storage_change: Option<&reth_trie::HashedStorage> = entry.2;

            let storage_root = if let Some(ss) = storage_change {
                // Storage touched — apply two-pass canonical update to the storage trie.
                let storage_trie = self
                    .storage_tries
                    .entry(*hashed_address)
                    .or_insert_with(|| arena::Mpt::new(self.bump));
                if ss.wiped {
                    *storage_trie = arena::Mpt::new(self.bump);
                }

                slot_inserts.clear();
                slot_deletes.clear();
                for (k, v) in ss.storage.iter() {
                    if v.is_zero() {
                        slot_deletes.push(*k);
                    } else {
                        slot_inserts.push((*k, *v));
                    }
                }
                slot_inserts.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                slot_deletes.sort_unstable();

                for (k, v) in &slot_inserts {
                    storage_trie.insert_rlp(k.as_slice(), *v).unwrap();
                }
                for k in &slot_deletes {
                    storage_trie.delete(k.as_slice()).unwrap();
                }

                storage_trie.hash()
            } else {
                // Storage NOT touched — read the pre-existing storage_root from the sorted
                // psr slice. Advance `psr_idx` until the key is >= hashed_address; if it
                // matches, use the cached storage_root; otherwise the account is new and its
                // storage_root is EMPTY (which is also what arena::Mpt::new(bump).hash() would
                // produce, matching the host's `to_arena_witness` for unrevealed accounts).
                //
                // SOUNDNESS: `psr` is host-controlled and not hash-committed. A tampered value
                // here flows into `state_account.storage_root` below, into the state-trie leaf,
                // and into the final state root — caught by the executor's
                // `MismatchedStateRoot` check. See the field doc + executor `SOUNDNESS:` tag.
                let target = hashed_address.as_slice();
                let mut found = reth_trie::EMPTY_ROOT_HASH;
                while psr_idx < psr_count {
                    let off = psr_idx * PSR_ENTRY;
                    let key = &psr[off..off + 32];
                    if key < target {
                        psr_idx += 1;
                    } else {
                        if key == target {
                            found = B256::from_slice(&psr[off + 32..off + 64]);
                        }
                        break;
                    }
                }
                found
            };

            let state_account = TrieAccount {
                nonce: account.nonce,
                balance: account.balance,
                storage_root,
                code_hash: account.get_bytecode_hash(),
            };
            self.state_trie.insert_rlp(hashed_address.as_slice(), state_account).unwrap();
        }

        // Pass 2: state-trie deletes (sorted).
        for entry in &accounts_sorted {
            if entry.1.is_none() {
                self.state_trie.delete(entry.0.as_slice()).unwrap();
            }
        }
    }

    /// Computes the state root.
    pub fn state_root(&self) -> B256 {
        self.state_trie.hash()
    }
}

impl core::fmt::Debug for EthereumState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut ds = f.debug_struct("EthereumState");
        ds.field("state_trie", &self.state_trie);

        // Use BTreeMap for stable ordering when printing
        let ordered: std::collections::BTreeMap<_, _> = self.storage_tries.iter().collect();
        ds.field("storage_tries", &ordered);
        ds.finish()
    }
}

#[cfg(test)]
mod arena_integration_tests {
    use alloy_primitives::{keccak256, U256};
    use bumpalo::Bump;
    use reth_trie::EMPTY_ROOT_HASH;

    use super::*;

    /// The arena witness path (EthereumState -> to_arena_witness -> decode -> ArenaTries) must
    /// reproduce the exact state/storage roots and account lookups of the legacy MptNode state.
    #[test]
    fn test_arena_witness_roundtrip_state() {
        // A storage trie for one account.
        let mut storage = MptNode::default();
        for i in 0..50u64 {
            storage.insert_rlp(keccak256(i.to_be_bytes()).as_slice(), U256::from(i + 1)).unwrap();
        }
        let storage_root = storage.hash();
        let addr_hash = keccak256([7u8; 20]);

        // A state trie of TrieAccount leaves; account 0 points at the storage trie above.
        let mut state_trie = MptNode::default();
        for i in 0..200u64 {
            let account = TrieAccount {
                nonce: i,
                balance: U256::from(i) * U256::from(1000u64),
                storage_root: if i == 0 { storage_root } else { EMPTY_ROOT_HASH },
                code_hash: keccak256([]),
            };
            let key = if i == 0 { addr_hash } else { keccak256(i.to_be_bytes()) };
            state_trie.insert_rlp(key.as_slice(), account).unwrap();
        }

        let mut storage_tries = HashMap::with_hasher(Default::default());
        storage_tries.insert(addr_hash, storage);
        let state = EthereumState { state_trie, storage_tries };
        let state_root = state.state_root();

        // Round-trip through the arena codec.
        let witness = state.to_arena_witness();
        let bump = Bump::new();
        let tries = ArenaTries::decode(&bump, &witness).unwrap();

        assert_eq!(tries.state_root(), state_root, "arena state root mismatch");
        assert_eq!(tries.storage_tries[&addr_hash].hash(), storage_root, "storage root mismatch");

        let legacy = state.state_trie.get_rlp::<TrieAccount>(addr_hash.as_slice()).unwrap();
        let arena = tries.state_trie.get_rlp::<TrieAccount>(addr_hash.as_slice()).unwrap();
        assert_eq!(legacy, arena, "account lookup mismatch");
    }

    /// Soundness regression test for the `pre_state_storage_roots` cache.
    ///
    /// The psr cache is **untrusted host input** — there's no separate Merkle commitment over
    /// its bytes. We rely on the executor's final `state_root != block.header.state_root`
    /// check to catch any tampering. This test makes that property executable: it
    ///
    ///   1. Builds an `EthereumState` where one account has non-empty storage_root and
    ///      another's state will be updated **without** touching its storage (the only path
    ///      that consults psr).
    ///   2. Encodes to arena witness, decodes, applies `update`, records the "honest" root.
    ///   3. Re-encodes, **flips bits in the psr entry's storage_root** for the affected
    ///      account, decodes the tampered blob, applies the same update, records the
    ///      "tampered" root.
    ///   4. Asserts that `honest_root != tampered_root` — i.e., any tamper in psr
    ///      deterministically changes the computed state root, which the executor's
    ///      `MismatchedStateRoot` check would surface as a hard error.
    ///
    /// If anyone weakens or removes the executor's final check, the soundness chain breaks
    /// and this test (or its property) needs re-evaluation. See the `SOUNDNESS:` tag at
    /// `crates/executor/client/src/executor.rs`.
    #[test]
    fn psr_tamper_changes_state_root() {
        // --- Set up: state_trie with two distinguishable accounts ---
        // `victim_hash` has non-empty storage_root and will be updated this block WITHOUT a
        // storage change → goes through psr fallback. `bystander_hash` is a second account
        // so the state_trie is non-trivial and the psr section has multiple entries.
        let mut storage = MptNode::default();
        for i in 0..10u64 {
            storage.insert_rlp(keccak256(i.to_be_bytes()).as_slice(), U256::from(i + 1)).unwrap();
        }
        let storage_root = storage.hash();
        assert_ne!(storage_root, EMPTY_ROOT_HASH, "test setup expects non-empty storage_root");

        let victim_hash = keccak256([1u8; 20]);
        let bystander_hash = keccak256([2u8; 20]);

        let mut state_trie = MptNode::default();
        state_trie
            .insert_rlp(
                victim_hash.as_slice(),
                TrieAccount {
                    nonce: 1,
                    balance: U256::from(1000u64),
                    storage_root,
                    code_hash: keccak256([]),
                },
            )
            .unwrap();
        state_trie
            .insert_rlp(
                bystander_hash.as_slice(),
                TrieAccount {
                    nonce: 0,
                    balance: U256::ZERO,
                    storage_root: EMPTY_ROOT_HASH,
                    code_hash: keccak256([]),
                },
            )
            .unwrap();

        let mut storage_tries = HashMap::with_hasher(Default::default());
        storage_tries.insert(victim_hash, storage);
        let state = EthereumState { state_trie, storage_tries };

        // --- The post-state change we'll apply: bump victim's nonce + balance, but no slot
        //     writes (so post_state.storages has no entry for victim → psr fallback path). ---
        let mut post_state = HashedPostState::default();
        post_state.accounts.insert(
            victim_hash,
            Some(reth_primitives_traits::Account {
                nonce: 2,
                balance: U256::from(2000u64),
                bytecode_hash: None,
            }),
        );

        // --- 1. Honest path: encode, decode, update, capture root ---
        let honest_blob = state.to_arena_witness();
        let bump_honest = Bump::new();
        let mut honest_tries = ArenaTries::decode(&bump_honest, &honest_blob).unwrap();
        honest_tries.update(&post_state);
        let honest_root = honest_tries.state_root();

        // --- 2. Tampered path: same blob, but flip every byte of victim's psr storage_root ---
        let mut tampered_blob = honest_blob.clone();
        let (psr_start, psr_count) = locate_psr_section(&tampered_blob);
        let mut found = false;
        for i in 0..psr_count {
            let off = psr_start + i * 64;
            if &tampered_blob[off..off + 32] == victim_hash.as_slice() {
                // Flip every byte of the cached storage_root — guarantees a different value.
                for b in &mut tampered_blob[off + 32..off + 64] {
                    *b ^= 0xff;
                }
                found = true;
                break;
            }
        }
        assert!(found, "expected victim's hashed_address to appear in the psr section");

        let bump_tampered = Bump::new();
        let mut tampered_tries = ArenaTries::decode(&bump_tampered, &tampered_blob).unwrap();
        tampered_tries.update(&post_state);
        let tampered_root = tampered_tries.state_root();

        // --- 3. The whole soundness argument in one line ---
        assert_ne!(
            honest_root, tampered_root,
            "psr tamper failed to change the computed state root — SOUNDNESS BROKEN. \
             The executor's `MismatchedStateRoot` check is the only thing that catches a \
             lying host; if tampering doesn't propagate to the root, the proof would pass with \
             a forged storage_root. Investigate before shipping.",
        );
    }

    /// Re-parse the arena-witness header just enough to find `(psr_start, psr_count)`.
    /// Mirrors the wire format documented on `EthereumState::to_arena_witness`.
    fn locate_psr_section(blob: &[u8]) -> (usize, usize) {
        let read_u32 = |b: &[u8], pos: &mut usize| -> usize {
            let v = u32::from_le_bytes(b[*pos..*pos + 4].try_into().unwrap()) as usize;
            *pos += 4;
            v
        };

        let mut pos = 0usize;
        // state_trie
        let _state_nodes = read_u32(blob, &mut pos);
        let state_len = read_u32(blob, &mut pos);
        pos += state_len;
        // storage_tries
        let num_storage = read_u32(blob, &mut pos);
        for _ in 0..num_storage {
            pos += 32; // hashed_address
            let _nodes = read_u32(blob, &mut pos);
            let len = read_u32(blob, &mut pos);
            pos += len;
        }
        // psr count + start offset
        let num_psr = read_u32(blob, &mut pos);
        (pos, num_psr)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FromProofError {
    #[error("Node {} is not found by hash", .0)]
    NodeNotFoundByHash(usize),
    #[error("Node {} refrences invalid successor", .0)]
    NodeHasInvalidSuccessor(usize),
    #[error("Node {} cannot have children and is invalid", .0)]
    NodeCannotHaveChildren(usize),
    #[error("Found mismatched storage root after reconstruction \n account {}, found {}, expected {}", .0, .1, .2)]
    MismatchedStorageRoot(Address, B256, B256),
    #[error("Found mismatched state root after reconstruction \n found {}, expected {}", .0, .1)]
    MismatchedStateRoot(B256, B256),
    // todo: Should decode return a decoder error?
    #[error("Error decoding proofs from bytes, {}", .0)]
    DecodingError(#[from] Error),
}
