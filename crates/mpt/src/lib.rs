#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_primitives::{keccak256, map::HashMap, Address, B256};
use alloy_rpc_types::EIP1186AccountProofResponse;
use reth_trie::{AccountProof, HashedPostState, HashedStorage, TrieAccount};
use serde::{Deserialize, Serialize};

#[cfg(feature = "execution-witness")]
mod execution_witness;

/// Module containing MPT code adapted from `zeth`.
mod mpt;
pub use mpt::Error;

/// Experimental arena-based, zero-copy MPT (ported from `openvm-eth`).
#[cfg(feature = "arena")]
pub mod arena;

use mpt::{
    mpt_from_proof, parse_proof, proofs_to_tries, resolve_nodes, transition_proofs_to_tries,
};

/// Legacy pointer-based MPT node. Re-exported (only with the `arena` feature) so it can be
/// benchmarked against the arena implementation.
#[cfg(feature = "arena")]
pub use mpt::MptNode;
#[cfg(not(feature = "arena"))]
use mpt::MptNode;

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
    pub fn update(&mut self, post_state: &HashedPostState) {
        for (hashed_address, account) in post_state.accounts.iter() {
            match account {
                Some(account) => {
                    let state_storage = &post_state
                        .storages
                        .get(hashed_address)
                        .cloned()
                        .unwrap_or_else(|| HashedStorage::new(false));
                    let storage_root = {
                        let storage_trie = self.storage_tries.entry(*hashed_address).or_default();

                        if state_storage.wiped {
                            storage_trie.clear();
                        }

                        for (key, value) in state_storage.storage.iter() {
                            let key = key.as_slice();
                            if value.is_zero() {
                                storage_trie.delete(key).unwrap();
                            } else {
                                storage_trie.insert_rlp(key, *value).unwrap();
                            }
                        }

                        storage_trie.hash()
                    };

                    let state_account = TrieAccount {
                        nonce: account.nonce,
                        balance: account.balance,
                        storage_root,
                        code_hash: account.get_bytecode_hash(),
                    };
                    self.state_trie.insert_rlp(hashed_address.as_slice(), state_account).unwrap();
                }
                None => {
                    self.state_trie.delete(hashed_address.as_slice()).unwrap();
                }
            }
        }
    }

    /// Computes the state root.
    pub fn state_root(&self) -> B256 {
        self.state_trie.hash()
    }

    /// Encodes the state into the arena codec for the zero-copy witness path: each trie is
    /// converted to an arena [`arena::Mpt`] and `encode_trie`d. The guest reconstructs the
    /// tries with [`ArenaStateWitness::decode`] in a single zero-copy, hash-verifying pass.
    #[cfg(feature = "arena")]
    pub fn to_arena_witness(&self) -> ArenaStateWitness {
        let encode = |node: &MptNode| {
            let bump = bumpalo::Bump::new();
            let trie = arena::Mpt::from_mpt_node(&bump, node);
            EncodedTrie { bytes: trie.encode_trie(), num_nodes: trie.num_nodes() }
        };

        ArenaStateWitness {
            state_trie: encode(&self.state_trie),
            storage_tries: self.storage_tries.iter().map(|(k, v)| (*k, encode(v))).collect(),
        }
    }
}

/// An arena-codec-encoded trie: the bytes produced by [`arena::Mpt::encode_trie`] plus the node
/// count needed by [`arena::Mpt::decode_trie`].
#[cfg(feature = "arena")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedTrie {
    pub bytes: Vec<u8>,
    pub num_nodes: usize,
}

/// The serialized witness state in the arena codec. This is what crosses the host->guest
/// boundary instead of the pointer-based [`EthereumState`]: it (de)serializes as plain byte
/// vectors (cheap bincode) and the guest decodes it zero-copy with hash verification folded in.
#[cfg(feature = "arena")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArenaStateWitness {
    pub state_trie: EncodedTrie,
    pub storage_tries: HashMap<B256, EncodedTrie>,
}

#[cfg(feature = "arena")]
impl ArenaStateWitness {
    /// Decodes the witness into arena tries borrowing `bump` (and `self`'s byte buffers) — one
    /// bump allocation, zero-copy, with each node's hash verified against its parent inline.
    pub fn decode<'a>(&'a self, bump: &'a bumpalo::Bump) -> Result<ArenaTries<'a>, arena::Error> {
        let decode = |t: &'a EncodedTrie| {
            let mut bytes = t.bytes.as_slice();
            arena::Mpt::decode_trie(bump, &mut bytes, t.num_nodes)
        };

        let state_trie = decode(&self.state_trie)?;
        let mut storage_tries = HashMap::with_hasher(Default::default());
        for (address, trie) in &self.storage_tries {
            storage_tries.insert(*address, decode(trie)?);
        }

        Ok(ArenaTries { bump, state_trie, storage_tries })
    }
}

/// Decoded arena tries, the guest-side equivalent of [`EthereumState`]. Borrows the bump arena
/// and the witness bytes for the duration of block execution; provides the same lookups,
/// `update` and `state_root` operations the executor needs.
#[cfg(feature = "arena")]
#[derive(Debug)]
pub struct ArenaTries<'a> {
    bump: &'a bumpalo::Bump,
    pub state_trie: arena::Mpt<'a>,
    pub storage_tries: HashMap<B256, arena::Mpt<'a>>,
}

#[cfg(feature = "arena")]
impl ArenaTries<'_> {
    /// Mutates state based on diffs provided in [`HashedPostState`] (mirrors
    /// [`EthereumState::update`] on the arena tries).
    pub fn update(&mut self, post_state: &HashedPostState) {
        for (hashed_address, account) in post_state.accounts.iter() {
            match account {
                Some(account) => {
                    let state_storage = &post_state
                        .storages
                        .get(hashed_address)
                        .cloned()
                        .unwrap_or_else(|| HashedStorage::new(false));
                    let storage_root = {
                        let storage_trie = self
                            .storage_tries
                            .entry(*hashed_address)
                            .or_insert_with(|| arena::Mpt::new(self.bump));

                        if state_storage.wiped {
                            *storage_trie = arena::Mpt::new(self.bump);
                        }

                        for (key, value) in state_storage.storage.iter() {
                            let key = key.as_slice();
                            if value.is_zero() {
                                storage_trie.delete(key).unwrap();
                            } else {
                                storage_trie.insert_rlp(key, *value).unwrap();
                            }
                        }

                        storage_trie.hash()
                    };

                    let state_account = TrieAccount {
                        nonce: account.nonce,
                        balance: account.balance,
                        storage_root,
                        code_hash: account.get_bytecode_hash(),
                    };
                    self.state_trie.insert_rlp(hashed_address.as_slice(), state_account).unwrap();
                }
                None => {
                    self.state_trie.delete(hashed_address.as_slice()).unwrap();
                }
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

#[cfg(all(test, feature = "arena"))]
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
        let tries = witness.decode(&bump).unwrap();

        assert_eq!(tries.state_root(), state_root, "arena state root mismatch");
        assert_eq!(tries.storage_tries[&addr_hash].hash(), storage_root, "storage root mismatch");

        let legacy = state.state_trie.get_rlp::<TrieAccount>(addr_hash.as_slice()).unwrap();
        let arena = tries.state_trie.get_rlp::<TrieAccount>(addr_hash.as_slice()).unwrap();
        assert_eq!(legacy, arena, "account lookup mismatch");
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
