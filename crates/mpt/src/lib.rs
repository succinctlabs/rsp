#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_primitives::{
    keccak256,
    map::{B256Set, HashMap},
    Address, B256, U256,
};
use alloy_rlp::Decodable;
use alloy_rpc_types::EIP1186AccountProofResponse;
use reth_trie::{
    bundle_state::BundleState, AccountProof, HashedPostState, KeccakKeyHasher, TrieAccount,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "execution-witness")]
mod execution_witness;

/// Module containing MPT code adapted from `zeth`.
mod mpt;
pub use mpt::Error;

/// Arena-based, zero-copy MPT used as the witness format on the host->guest boundary
/// (ported from `openvm-eth`). This is the alternative backend to the pointer-based
/// [`MptNode`] state, selected via the `arena` cargo feature on the client/host binaries.
pub mod arena;

use mpt::{
    mpt_from_proof, node_from_digest, parse_proof, proofs_to_tries, resolve_nodes,
    transition_proofs_to_tries, EMPTY_ROOT,
};

/// Legacy pointer-based MPT node — used host-side to build the witness from RPC proofs before
/// re-encoding into the arena codec. In the arena guest path it is not used (the guest only
/// sees the arena form); in the legacy guest path the whole [`EthereumState`] is deserialized
/// directly.
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

    /// Applies a complete hashed diff containing explicit deletions for every old storage slot.
    /// For stateless execution output, use [`StateTries::update_from_bundle`].
    pub fn update(&mut self, post_state: &HashedPostState) {
        self.update_with_storage_wipes(post_state, &B256Set::default());
    }

    /// Applies a hashed diff, clearing storage for each wiped account before inserting its slots.
    pub fn update_with_storage_wipes(
        &mut self,
        post_state: &HashedPostState,
        wiped_storage: &B256Set,
    ) {
        for (hashed_address, account) in post_state.accounts.iter() {
            match account {
                Some(account) => {
                    let storage_root = {
                        let storage_trie =
                            self.storage_tries.entry(*hashed_address).or_insert_with(|| {
                                node_from_digest(account_storage_root_or_empty(
                                    &self.state_trie,
                                    hashed_address,
                                ))
                            });

                        if wiped_storage.contains(hashed_address) {
                            storage_trie.clear();
                        }

                        if let Some(storage) = post_state.storages.get(hashed_address) {
                            for (key, value) in &storage.storage {
                                let key = key.as_slice();
                                if value.is_zero() {
                                    storage_trie.delete(key).unwrap();
                                } else {
                                    storage_trie.insert_rlp(key, *value).unwrap();
                                }
                            }
                        }

                        storage_trie.hash()
                    };

                    let state_account = account.to_owned().into_trie_account(storage_root);
                    self.state_trie.insert_rlp(hashed_address.as_slice(), state_account).unwrap();
                }
                None => {
                    self.state_trie.delete(hashed_address.as_slice()).unwrap();
                    self.storage_tries.remove(hashed_address);
                }
            }
        }
    }

    /// Computes the state root.
    pub fn state_root(&self) -> B256 {
        self.state_trie.hash()
    }

    /// Encodes the state into a single flat byte blob in the arena codec — the zero-copy
    /// witness used by the `arena` guest path. All tries are `encode_trie`d and concatenated
    /// with framing (num_nodes/length/key), and the guest reconstructs them zero-copy by
    /// borrowing the blob directly with [`ArenaTries::decode`].
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
    /// The trailing root cache is emitted for compatibility with older readers.
    /// Current readers derive storage roots from the account leaves in the state trie.
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

        // Preserve the existing wire format for consumers that read this section.
        let mut psr: Vec<(B256, B256)> = Vec::new();
        self.state_trie.for_each_leaves(|key, mut value| {
            let account = TrieAccount::decode(&mut value).unwrap();
            psr.push((B256::from_slice(key), account.storage_root));
        });
        psr.sort_unstable_by_key(|entry| entry.0);
        put_u32(&mut blob, psr.len());
        for (hashed_address, storage_root) in &psr {
            blob.extend_from_slice(hashed_address.as_slice());
            blob.extend_from_slice(storage_root.as_slice());
        }
        blob
    }
}

/// The `storage_root` in the account leaf of `hashed_address`, or the empty root if the account
/// does not exist.
fn account_storage_root_or_empty(state_trie: &MptNode, hashed_address: &B256) -> B256 {
    state_trie
        .get_rlp::<TrieAccount>(hashed_address.as_slice())
        .expect("updated account leaf must be resolved")
        .map_or(EMPTY_ROOT, |account| account.storage_root)
}

/// Decoded arena tries — the guest-side equivalent of [`EthereumState`] for the `arena` path.
/// Borrows the bump and the witness blob for the duration of block execution; provides
/// account/storage lookups, `update`, and `state_root`.
#[derive(Debug)]
pub struct ArenaTries<'a> {
    bump: &'a bumpalo::Bump,
    pub state_trie: arena::Mpt<'a>,
    pub storage_tries: HashMap<B256, arena::Mpt<'a>>,
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

        // Check the legacy section's framing, but never use it as account state.
        let count = read_u32(blob, &mut pos);
        let len = count.checked_mul(PSR_ENTRY).ok_or(alloy_rlp::Error::InputTooShort)?;
        blob[pos..].get(..len).ok_or(alloy_rlp::Error::InputTooShort)?;

        Ok(ArenaTries { bump, state_trie, storage_tries })
    }
}

impl ArenaTries<'_> {
    fn account_storage_root(&self, hashed_address: B256) -> B256 {
        self.account(hashed_address).map_or(EMPTY_ROOT, |account| account.storage_root)
    }

    /// Applies a complete hashed diff, as in [`EthereumState::update`].
    /// For stateless execution output, use [`StateTries::update_from_bundle`].
    ///
    /// Applies changes in the canonical-witness-spec order (keys sorted lexicographically
    /// ascending, inserts/updates before deletes, at both storage and state-trie levels). This
    /// order is what the `Canonical` `ExecutionWitnessMode` assumes when emitting siblings;
    /// legacy mode tolerates any order (its sibling set is a superset), so the single
    /// implementation works for both witness shapes.
    pub fn update(&mut self, post_state: &HashedPostState) {
        self.update_with_storage_wipes(post_state, &B256Set::default());
    }

    /// Applies a hashed diff, clearing storage for each wiped account before inserting its slots.
    pub fn update_with_storage_wipes(
        &mut self,
        post_state: &HashedPostState,
        wiped_storage: &B256Set,
    ) {
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

        // Pass 1: storage tries + state-trie inserts/updates (sorted).
        for entry in &accounts_sorted {
            let hashed_address: &B256 = entry.0;
            let Some(account) = entry.1 else { continue };
            let wiped = wiped_storage.contains(hashed_address);
            let storage_change = entry.2.filter(|ss| !ss.storage.is_empty());

            let storage_root = if wiped || storage_change.is_some() {
                if !wiped && !self.storage_tries.contains_key(hashed_address) {
                    assert_eq!(
                        self.account_storage_root(*hashed_address),
                        EMPTY_ROOT,
                        "nonempty account storage must be witnessed"
                    );
                }
                // Storage touched — apply two-pass canonical update to the storage trie.
                let storage_trie = self
                    .storage_tries
                    .entry(*hashed_address)
                    .or_insert_with(|| arena::Mpt::new(self.bump));
                if wiped {
                    *storage_trie = arena::Mpt::new(self.bump);
                }

                slot_inserts.clear();
                slot_deletes.clear();
                if let Some(storage) = storage_change {
                    for (k, v) in &storage.storage {
                        if v.is_zero() {
                            slot_deletes.push(*k);
                        } else {
                            slot_inserts.push((*k, *v));
                        }
                    }
                }
                slot_inserts.sort_unstable_by_key(|entry| entry.0);
                slot_deletes.sort_unstable();

                for (k, v) in &slot_inserts {
                    storage_trie.insert_rlp(k.as_slice(), *v).unwrap();
                }
                for k in &slot_deletes {
                    storage_trie.delete(k.as_slice()).unwrap();
                }

                storage_trie.hash()
            } else {
                self.account_storage_root(*hashed_address)
            };

            let state_account = account.to_owned().into_trie_account(storage_root);
            self.state_trie.insert_rlp(hashed_address.as_slice(), state_account).unwrap();
        }

        // Pass 2: state-trie deletes (sorted).
        for entry in &accounts_sorted {
            if entry.1.is_none() {
                self.state_trie.delete(entry.0.as_slice()).unwrap();
                self.storage_tries.remove(entry.0);
            }
        }
    }

    /// Computes the state root.
    pub fn state_root(&self) -> B256 {
        self.state_trie.hash()
    }
}

/// Backend-agnostic view over the Ethereum state + storage tries consumed by the client
/// executor. Implemented by both the pointer-based [`EthereumState`] (legacy, the default
/// backend) and the arena-based [`ArenaTries`] (selected via the `arena` cargo feature on the
/// client/host binaries).
///
/// This mirrors the trie seam of `paradigmxyz/stateless`'s `StatelessTrie`, adapted to rsp's
/// witness formats: construction differs per backend (bincode-`EthereumState` vs the flat arena
/// blob), so it stays backend-specific and is not part of this trait — only the read/verify/
/// update operations the executor drives generically live here.
pub trait StateTries {
    /// Root hash of the state trie.
    fn state_root(&self) -> B256;

    /// Account at `hashed_address` (`keccak256(address)`), if revealed by the witness.
    fn account(&self, hashed_address: B256) -> Option<TrieAccount>;

    /// Storage slot value at `(hashed_address, hashed_slot)`, where `hashed_slot =
    /// keccak256(slot_key)`. Returns zero for an unset slot.
    fn storage_value(&self, hashed_address: B256, hashed_slot: B256) -> U256;

    /// Iterator of `(hashed_address, storage_root)` over every revealed storage trie — used
    /// during witness-db construction to verify each storage root against its account leaf.
    fn storage_roots(&self) -> impl Iterator<Item = (B256, B256)> + '_;

    /// Applies the post-execution [`HashedPostState`] diff to the tries.
    fn update(&mut self, post_state: &HashedPostState) {
        self.update_with_storage_wipes(post_state, &B256Set::default());
    }

    /// Applies a hashed diff with a set of accounts whose old storage must be discarded.
    fn update_with_storage_wipes(&mut self, post_state: &HashedPostState, wiped_storage: &B256Set);

    /// Applies execution output without enumerating untouched slots from a full state provider.
    fn update_from_bundle(&mut self, bundle: &BundleState) {
        // Reth's hashed bundle contains only the slots held in memory. After a destruction and
        // recreation, the account can have old slots that were never read by the executor.
        let wiped_storage = bundle
            .state
            .iter()
            .filter(|(_, account)| account.was_destroyed() && account.info.is_some())
            .map(|(address, _)| keccak256(address))
            .collect();
        let post_state = HashedPostState::from_bundle_state::<KeccakKeyHasher>(&bundle.state);
        self.update_with_storage_wipes(&post_state, &wiped_storage);
    }
}

impl StateTries for EthereumState {
    fn state_root(&self) -> B256 {
        // Inherent methods take precedence, so this dispatches to `EthereumState::state_root`.
        EthereumState::state_root(self)
    }

    fn account(&self, hashed_address: B256) -> Option<TrieAccount> {
        self.state_trie.get_rlp::<TrieAccount>(hashed_address.as_slice()).unwrap()
    }

    fn storage_value(&self, hashed_address: B256, hashed_slot: B256) -> U256 {
        let storage_trie = self
            .storage_tries
            .get(&hashed_address)
            .expect("A storage trie must be provided for each account");
        storage_trie
            .get_rlp::<U256>(hashed_slot.as_slice())
            .expect("Can get from MPT")
            .unwrap_or_default()
    }

    fn storage_roots(&self) -> impl Iterator<Item = (B256, B256)> + '_ {
        self.storage_tries.iter().map(|(hashed_address, trie)| (*hashed_address, trie.hash()))
    }

    fn update_with_storage_wipes(&mut self, post_state: &HashedPostState, wiped_storage: &B256Set) {
        EthereumState::update_with_storage_wipes(self, post_state, wiped_storage)
    }
}

impl StateTries for ArenaTries<'_> {
    fn state_root(&self) -> B256 {
        ArenaTries::state_root(self)
    }

    fn account(&self, hashed_address: B256) -> Option<TrieAccount> {
        self.state_trie.get_rlp::<TrieAccount>(hashed_address.as_slice()).unwrap()
    }

    fn storage_value(&self, hashed_address: B256, hashed_slot: B256) -> U256 {
        let Some(storage_trie) = self.storage_tries.get(&hashed_address) else {
            assert_eq!(
                self.account_storage_root(hashed_address),
                EMPTY_ROOT,
                "nonempty account storage must be witnessed"
            );
            return U256::ZERO;
        };
        storage_trie
            .get_rlp::<U256>(hashed_slot.as_slice())
            .expect("Can get from MPT")
            .unwrap_or_default()
    }

    fn storage_roots(&self) -> impl Iterator<Item = (B256, B256)> + '_ {
        self.storage_tries.iter().map(|(hashed_address, trie)| (*hashed_address, trie.hash()))
    }

    fn update_with_storage_wipes(&mut self, post_state: &HashedPostState, wiped_storage: &B256Set) {
        ArenaTries::update_with_storage_wipes(self, post_state, wiped_storage)
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
}

#[cfg(test)]
mod update_tests {
    use alloy_primitives::{keccak256, B256, U256};
    use reth_primitives_traits::Account;
    use reth_trie::{HashedPostState, HashedStorage, TrieAccount, EMPTY_ROOT_HASH};

    use super::*;

    fn slot(i: u64) -> B256 {
        keccak256(i.to_be_bytes())
    }

    fn leaf(nonce: u64, balance: u64, storage_root: B256) -> TrieAccount {
        TrieAccount { nonce, balance: U256::from(balance), storage_root, code_hash: keccak256([]) }
    }

    /// A state trie holding a contract with ten storage slots and an EOA with empty storage,
    /// returned with the contract's hashed address and storage root. `include_contract_trie`
    /// selects whether `storage_tries` carries the contract's trie.
    fn fixture(include_contract_trie: bool) -> (EthereumState, B256, B256) {
        let mut storage = MptNode::default();
        for i in 0..10u64 {
            storage.insert_rlp(slot(i).as_slice(), U256::from(i + 1)).unwrap();
        }
        let storage_root = storage.hash();
        let contract = keccak256([1u8; 20]);

        let mut state_trie = MptNode::default();
        state_trie.insert_rlp(contract.as_slice(), leaf(1, 1000, storage_root)).unwrap();
        state_trie
            .insert_rlp(keccak256([2u8; 20]).as_slice(), leaf(0, 0, EMPTY_ROOT_HASH))
            .unwrap();

        let mut storage_tries = HashMap::with_hasher(Default::default());
        if include_contract_trie {
            storage_tries.insert(contract, storage);
        }
        (EthereumState { state_trie, storage_tries }, contract, storage_root)
    }

    fn post(hashed_address: B256, nonce: u64, balance: u64) -> HashedPostState {
        let mut post_state = HashedPostState::default();
        post_state.accounts.insert(
            hashed_address,
            Some(Account { nonce, balance: U256::from(balance), bytecode_hash: None }),
        );
        post_state
    }

    fn storage_root_of(state: &EthereumState, hashed_address: B256) -> B256 {
        state
            .state_trie
            .get_rlp::<TrieAccount>(hashed_address.as_slice())
            .unwrap()
            .unwrap()
            .storage_root
    }

    fn update_both(state: &mut EthereumState, post_state: &HashedPostState) {
        let witness = state.to_arena_witness();
        let bump = bumpalo::Bump::new();
        let mut arena = ArenaTries::decode(&bump, &witness).unwrap();
        arena.update(post_state);
        state.update(post_state);
        assert_eq!(arena.state_root(), state.state_root());
    }

    #[test]
    fn arena_reads_zero_for_proven_empty_or_absent_accounts() {
        let (state, _, _) = fixture(false);
        let witness = state.to_arena_witness();
        let bump = bumpalo::Bump::new();
        let arena = ArenaTries::decode(&bump, &witness).unwrap();
        for address in [keccak256([2u8; 20]), keccak256([3u8; 20])] {
            assert_eq!(arena.storage_value(address, slot(0)), U256::ZERO);
        }
    }

    #[test]
    fn arena_requires_storage_for_reads_and_partial_updates() {
        let (state, contract, _) = fixture(false);
        let witness = state.to_arena_witness();
        let bump = bumpalo::Bump::new();
        let mut arena = ArenaTries::decode(&bump, &witness).unwrap();
        let mut post_state = post(contract, 2, 2000);
        post_state.storages.insert(contract, HashedStorage::from_iter([(slot(1), U256::ONE)]));
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || arena.storage_value(contract, slot(1))
        ))
        .is_err());
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || arena.update(&post_state)
        ))
        .is_err());
    }

    #[test]
    fn arena_preserves_the_leaf_root_without_the_legacy_cache() {
        let (mut state, contract, _) = fixture(true);
        let mut witness = state.to_arena_witness();
        // This fixture has two revealed accounts. Supply an empty legacy cache instead.
        witness.truncate(witness.len() - 2 * PSR_ENTRY - 4);
        witness.extend_from_slice(&0u32.to_le_bytes());
        let bump = bumpalo::Bump::new();
        let mut arena = ArenaTries::decode(&bump, &witness).unwrap();
        let mut post_state = post(contract, 2, 2000);
        post_state.storages.insert(contract, HashedStorage::default());
        state.update(&post_state);
        arena.update(&post_state);
        assert_eq!(arena.state_root(), state.state_root());
    }

    /// An account whose storage is not in the diff keeps the `storage_root` already in its leaf,
    /// with or without its storage trie in `storage_tries`.
    #[test]
    fn untouched_storage_keeps_its_root() {
        let (mut with_trie, contract, contract_root) = fixture(true);
        let (mut without_trie, _, _) = fixture(false);
        let post_state = post(contract, 2, 2000);

        update_both(&mut with_trie, &post_state);
        update_both(&mut without_trie, &post_state);

        assert_eq!(storage_root_of(&without_trie, contract), contract_root);
        assert_eq!(without_trie.state_root(), with_trie.state_root());
        let placeholder = &without_trie.storage_tries[&contract];
        assert!(placeholder.is_digest());
        assert_eq!(placeholder.hash(), contract_root);
    }

    #[test]
    #[should_panic(expected = "NodeNotResolved")]
    fn slot_write_without_a_supplied_trie_panics() {
        let (mut state, contract, _) = fixture(false);
        let mut post_state = post(contract, 2, 2000);
        post_state
            .storages
            .insert(contract, HashedStorage::from_iter([(slot(99), U256::from(7u64))]));
        state.update(&post_state);
    }

    #[test]
    fn empty_storage_takes_slot_writes_without_a_supplied_trie() {
        let (mut state, _, _) = fixture(true);
        let created = keccak256([3u8; 20]);
        let eoa = keccak256([2u8; 20]);
        let slots = [(slot(1), U256::from(11u64)), (slot(2), U256::from(22u64))];
        let mut expected = MptNode::default();
        for (key, value) in &slots {
            expected.insert_rlp(key.as_slice(), *value).unwrap();
        }

        let mut post_state = post(created, 1, 0);
        post_state
            .accounts
            .insert(eoa, Some(Account { nonce: 1, balance: U256::ZERO, bytecode_hash: None }));
        post_state.storages.insert(created, HashedStorage::from_iter(slots));
        post_state.storages.insert(eoa, HashedStorage::from_iter(slots));
        update_both(&mut state, &post_state);

        assert_eq!(storage_root_of(&state, created), expected.hash());
        assert_eq!(storage_root_of(&state, eoa), expected.hash());
    }

    #[test]
    fn deleting_an_account_discards_its_storage_before_later_creation() {
        let (mut state, contract, _) = fixture(true);
        let witness = state.to_arena_witness();
        let bump = bumpalo::Bump::new();
        let mut arena = ArenaTries::decode(&bump, &witness).unwrap();
        let mut deletion = HashedPostState::default();
        deletion.accounts.insert(contract, None);
        let creation = post(contract, 1, 0);
        for diff in [&deletion, &creation] {
            state.update(diff);
            arena.update(diff);
            assert_eq!(arena.state_root(), state.state_root());
        }
        assert_eq!(storage_root_of(&state, contract), EMPTY_ROOT_HASH);
    }

    #[test]
    fn recreated_account_uses_only_post_destruction_slots() {
        use reth_trie::bundle_state::{AccountStatus, BundleAccount, StorageSlot};

        let address = Address::repeat_byte(1);
        let key = U256::from(42);
        for new_value in [None, Some(U256::ONE)] {
            for include_contract_trie in [false, true] {
                let (mut state, contract, _) = fixture(include_contract_trie);
                let witness = state.to_arena_witness();
                let bump = bumpalo::Bump::new();
                let mut arena = ArenaTries::decode(&bump, &witness).unwrap();

                let mut bundle = BundleState::default();
                let storage = new_value
                    .into_iter()
                    .map(|v| (key, StorageSlot::new_changed(U256::ZERO, v)))
                    .collect();
                let account = BundleAccount::new(
                    Some(Default::default()),
                    Some(Default::default()),
                    storage,
                    AccountStatus::DestroyedChanged,
                );
                bundle.state.insert(address, account);
                arena.update_from_bundle(&bundle);
                state.update_from_bundle(&bundle);

                let mut expected = MptNode::default();
                if let Some(value) = new_value {
                    expected.insert_rlp(keccak256(B256::from(key)).as_slice(), value).unwrap();
                }
                assert_eq!(storage_root_of(&state, contract), expected.hash());
                assert_eq!(arena.state_root(), state.state_root());
            }
        }
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
