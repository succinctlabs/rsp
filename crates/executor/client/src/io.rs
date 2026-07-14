use std::iter::once;

use alloy_consensus::{Block, BlockHeader, Header};
use alloy_primitives::map::HashMap;
use itertools::Itertools;
use reth_errors::ProviderError;
use reth_ethereum_primitives::EthPrimitives;
use reth_primitives_traits::{NodePrimitives, SealedHeader};
use reth_trie::EMPTY_ROOT_HASH;
use revm::{
    state::{AccountInfo, Bytecode},
    DatabaseRef,
};
use revm_primitives::{keccak256, Address, B256, U256};
use rsp_mpt::StateTries;
use rsp_primitives::genesis::Genesis;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

#[cfg(not(feature = "arena"))]
use rsp_mpt::EthereumState;
#[cfg(feature = "arena")]
use {bumpalo::Bump, rsp_mpt::ArenaTries};

use crate::error::ClientError;

/// Bincode-compatible serde for [`Block`] via RLP.
///
/// The default serde representation of [`Block`] is not bincode-compatible (the inner transaction
/// envelope serializes via `#[serde(flatten)]`, which makes bincode fail with
/// `SequenceMustHaveLength`). reth makes blocks bincode-serializable by round-tripping through RLP
/// bytes; we mirror that here because the previous
/// `reth_primitives_traits::serde_bincode_compat::Block` wrapper was removed when
/// `reth-primitives-traits` was published to crates.io.
mod block_rlp {
    use alloy_consensus::Block;
    use alloy_primitives::Bytes;
    use alloy_rlp::{Decodable, Encodable};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_with::{DeserializeAs, SerializeAs};

    pub(crate) struct BlockRlp;

    impl<T: Encodable> SerializeAs<Block<T>> for BlockRlp {
        fn serialize_as<S: Serializer>(
            source: &Block<T>,
            serializer: S,
        ) -> Result<S::Ok, S::Error> {
            Bytes::from(alloy_rlp::encode(source)).serialize(serializer)
        }
    }

    impl<'de, T: Decodable> DeserializeAs<'de, Block<T>> for BlockRlp {
        fn deserialize_as<D: Deserializer<'de>>(deserializer: D) -> Result<Block<T>, D::Error> {
            let bytes = Bytes::deserialize(deserializer)?;
            Block::<T>::decode(&mut bytes.as_ref()).map_err(serde::de::Error::custom)
        }
    }
}

pub type EthClientExecutorInput = ClientExecutorInput<EthPrimitives>;

/// The input for the client to execute a block and fully verify the STF (state transition
/// function).
///
/// Instead of passing in the entire state, we only pass in the state roots along with merkle proofs
/// for the storage slots that were modified and accessed.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientExecutorInput<P: NodePrimitives> {
    /// The current block (which will be executed inside the client).
    #[serde_as(as = "block_rlp::BlockRlp")]
    pub current_block: Block<P::SignedTx>,
    /// The previous block headers starting from the most recent. There must be at least one header
    /// to provide the parent state root.
    #[serde_as(as = "Vec<alloy_consensus::serde_bincode_compat::Header>")]
    pub ancestor_headers: Vec<Header>,
    /// Network state as of the parent block.
    ///
    /// - default (legacy) backend: the full [`EthereumState`] (pointer-based MPT), serialized
    ///   inline via bincode.
    /// - `arena` backend: the arena-codec witness blob, shipped as a *separate* SP1 stdin item and
    ///   `#[serde(skip)]`ped here so bincode only handles the small header; the guest fills this
    ///   field from `sp1_zkvm::io::read_vec()` before executing and decodes it zero-copy (see
    ///   [`ArenaTries::decode`]).
    #[cfg(not(feature = "arena"))]
    pub parent_state: EthereumState,
    #[cfg(feature = "arena")]
    #[serde(skip, default)]
    pub parent_state: Vec<u8>,
    /// Account bytecodes.
    pub bytecodes: Vec<Bytecode>,
    /// The genesis block, as a json string.
    pub genesis: Genesis,
    /// The genesis block, as a json string.
    pub custom_beneficiary: Option<Address>,
    /// Whether to track the cycle count of opcodes.
    pub opcode_tracking: bool,
}

impl<P: NodePrimitives> ClientExecutorInput<P> {
    /// Gets the immediate parent block's header.
    #[inline(always)]
    pub fn parent_header(&self) -> &Header {
        &self.ancestor_headers[0]
    }

    /// Creates a [`TrieDB`] backed by the legacy pointer-based [`EthereumState`].
    #[cfg(not(feature = "arena"))]
    #[inline(always)]
    pub fn witness_db(
        &self,
        sealed_headers: &[SealedHeader],
    ) -> Result<TrieDB<'_, EthereumState>, ClientError> {
        build_trie_db(&self.parent_state, self.state_anchor(), self.bytecodes(), sealed_headers)
    }

    /// Decodes the arena witness blob into bump-scoped tries (zero-copy, hash-verifying). The
    /// bump and returned tries must outlive the [`TrieDB`] built from them by
    /// [`Self::witness_db`].
    #[cfg(feature = "arena")]
    #[inline(always)]
    pub fn tries<'a>(&'a self, bump: &'a Bump) -> Result<ArenaTries<'a>, ClientError> {
        ArenaTries::decode(bump, &self.parent_state).map_err(|_| ClientError::MismatchedStateRoot)
    }

    /// Creates a [`TrieDB`] backed by the arena tries decoded via [`Self::tries`].
    #[cfg(feature = "arena")]
    #[inline(always)]
    pub fn witness_db<'a, 'b>(
        &'a self,
        tries: &'a ArenaTries<'b>,
        sealed_headers: &[SealedHeader],
    ) -> Result<TrieDB<'a, ArenaTries<'b>>, ClientError> {
        build_trie_db(tries, self.state_anchor(), self.bytecodes(), sealed_headers)
    }
}

impl<P: NodePrimitives> WitnessInput for ClientExecutorInput<P> {
    #[inline(always)]
    fn state_anchor(&self) -> B256 {
        self.parent_header().state_root()
    }

    #[inline(always)]
    fn bytecodes(&self) -> impl Iterator<Item = &Bytecode> {
        self.bytecodes.iter()
    }

    #[inline(always)]
    fn sealed_headers(&self) -> impl Iterator<Item = SealedHeader> {
        once(SealedHeader::seal_slow(self.current_block.header.clone()))
            .chain(self.ancestor_headers.iter().map(|h| SealedHeader::seal_slow(h.clone())))
    }
}

// The headed committed at the end of execution
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommittedHeader {
    #[serde_as(as = "alloy_consensus::serde_bincode_compat::Header")]
    pub header: Header,
}

impl From<Header> for CommittedHeader {
    fn from(header: Header) -> Self {
        CommittedHeader { header }
    }
}

/// Witness-backed database revm reads from during execution. Generic over the state-trie backend
/// `T` (the legacy [`EthereumState`] or the arena [`ArenaTries`]) via the [`StateTries`] trait.
#[derive(Debug)]
pub struct TrieDB<'a, T: StateTries> {
    inner: &'a T,
    block_hashes: HashMap<u64, B256>,
    bytecode_by_hash: HashMap<B256, &'a Bytecode>,
}

impl<'a, T: StateTries> TrieDB<'a, T> {
    pub fn new(
        inner: &'a T,
        block_hashes: HashMap<u64, B256>,
        bytecode_by_hash: HashMap<B256, &'a Bytecode>,
    ) -> Self {
        Self { inner, block_hashes, bytecode_by_hash }
    }
}

impl<T: StateTries> DatabaseRef for TrieDB<'_, T> {
    /// The database error type.
    type Error = ProviderError;

    /// Get basic account information.
    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let account_in_trie = self.inner.account(keccak256(address));

        let account = account_in_trie.map(|account_in_trie| AccountInfo {
            balance: account_in_trie.balance,
            nonce: account_in_trie.nonce,
            code_hash: account_in_trie.code_hash,
            account_id: None,
            code: None,
        });

        Ok(account)
    }

    /// Get account code by its hash.
    fn code_by_hash_ref(&self, hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(self.bytecode_by_hash.get(&hash).map(|code| (*code).clone()).unwrap())
    }

    /// Get storage value of address at index.
    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self.inner.storage_value(keccak256(address), keccak256(index.to_be_bytes::<32>())))
    }

    /// Get block hash by block number.
    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        Ok(*self
            .block_hashes
            .get(&number)
            .expect("A block hash must be provided for each block number"))
    }
}

/// Verifies the state/storage roots, ancestor headers and account bytecodes of a [`StateTries`]
/// backend, and constructs the [`TrieDB`] revm reads against during execution.
///
/// NOTE: For some unknown reasons, calling this via a bare trait method (rather than from a method
/// on the input type) causes a zkVM run to cost over 5M cycles more. The per-backend `witness_db`
/// inherent methods on [`ClientExecutorInput`] preserve that call shape.
#[inline(always)]
fn build_trie_db<'a, T: StateTries>(
    tries: &'a T,
    state_anchor: B256,
    bytecodes: impl Iterator<Item = &'a Bytecode>,
    sealed_headers: &[SealedHeader],
) -> Result<TrieDB<'a, T>, ClientError> {
    if state_anchor != tries.state_root() {
        return Err(ClientError::MismatchedStateRoot);
    }

    for (hashed_address, storage_root) in tries.storage_roots() {
        let account_storage_root =
            tries.account(hashed_address).map_or(EMPTY_ROOT_HASH, |a| a.storage_root);
        if account_storage_root != storage_root {
            return Err(ClientError::MismatchedStorageRoot);
        }
    }

    let bytecodes_by_hash =
        bytecodes.map(|code| (code.hash_slow(), code)).collect::<HashMap<_, _>>();

    // Verify and build block hashes
    let mut block_hashes: HashMap<u64, B256> = HashMap::with_hasher(Default::default());
    for (child_header, parent_header) in sealed_headers.iter().tuple_windows() {
        if parent_header.number() != child_header.number() - 1 {
            return Err(ClientError::InvalidHeaderBlockNumber(
                parent_header.number() + 1,
                child_header.number(),
            ));
        }

        let parent_header_hash = parent_header.hash();
        if parent_header_hash != child_header.parent_hash() {
            return Err(ClientError::InvalidHeaderParentHash(
                parent_header_hash,
                child_header.parent_hash(),
            ));
        }

        block_hashes.insert(parent_header.number(), child_header.parent_hash());
    }

    Ok(TrieDB::new(tries, block_hashes, bytecodes_by_hash))
}

/// A trait for the backend-independent inputs used to construct a [`TrieDB`].
pub trait WitnessInput {
    /// Gets the state trie root hash that the backing state must conform to.
    fn state_anchor(&self) -> B256;

    /// Gets an iterator over account bytecodes.
    fn bytecodes(&self) -> impl Iterator<Item = &Bytecode>;

    /// Gets an iterator over references to a consecutive, reverse-chronological block headers
    /// starting from the current block header.
    fn sealed_headers(&self) -> impl Iterator<Item = SealedHeader>;
}
