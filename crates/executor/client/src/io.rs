use std::iter::once;

use alloy_consensus::{
    serde_bincode_compat::{EthereumTxEnvelope as TxEnvelopeBincode, Header as HeaderBincode},
    Block, BlockBody, BlockHeader, Header, TxEip4844,
};
use alloy_eips::eip4895::Withdrawals;
use bumpalo::Bump;
use itertools::Itertools;
use reth_errors::ProviderError;
use reth_ethereum_primitives::{EthPrimitives, TransactionSigned};
use reth_primitives_traits::{NodePrimitives, SealedHeader};
use reth_trie::{TrieAccount, EMPTY_ROOT_HASH};
use revm::{
    state::{AccountInfo, Bytecode},
    DatabaseRef,
};
use revm_primitives::{keccak256, map::HashMap, Address, B256, U256};
use rsp_mpt::ArenaTries;
use rsp_primitives::genesis::Genesis;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_with::serde_as;

use crate::error::ClientError;

pub type EthClientExecutorInput = ClientExecutorInput<EthPrimitives>;

/// Bincode-compatible serialization for `Block<TransactionSigned>`.
///
/// Why this is needed: alloy v2's `Signed<T, Sig>` (the wrapper underneath every transaction
/// variant) has a manual `Serialize` impl that uses `#[serde(flatten)]` on the signature
/// field. `flatten` resolves to `Serializer::collect_map(None, ..)`, which bincode 1.x
/// rejects with "sequences and maps that have a knowable size ahead of time" — bincode 1's
/// binary format requires length prefixes.
///
/// `alloy_consensus` ships explicit `serde_bincode_compat::EthereumTxEnvelope` and `::Header`
/// wrappers that unflatten the offending fields into named struct fields; we just stitch
/// them together at the Block level (which alloy itself no longer ships, since reth's own
/// bincode paths don't serialize `Block` — only `ExecutionOutcome`, `Chain`, `TrieUpdates`,
/// etc., each of which has its own compat wrapper in reth).
///
/// This restores the pre-revm-38 deserialize-inputs cycle cost; the prior RLP-encode-then-
/// bincode-as-bytes workaround paid ~2M extra cycles for the guest-side RLP decode of every
/// transaction in the block.
fn serialize_block_bincode_compat<S>(
    block: &Block<TransactionSigned>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    #[serde_as]
    #[derive(Serialize)]
    struct Helper<'a> {
        #[serde_as(as = "HeaderBincode")]
        header: &'a Header,
        #[serde_as(as = "Vec<TxEnvelopeBincode<'_, TxEip4844>>")]
        transactions: &'a Vec<TransactionSigned>,
        #[serde_as(as = "Vec<HeaderBincode>")]
        ommers: &'a Vec<Header>,
        withdrawals: &'a Option<Withdrawals>,
    }

    Helper {
        header: &block.header,
        transactions: &block.body.transactions,
        ommers: &block.body.ommers,
        withdrawals: &block.body.withdrawals,
    }
    .serialize(serializer)
}

fn deserialize_block_bincode_compat<'de, D>(
    deserializer: D,
) -> Result<Block<TransactionSigned>, D::Error>
where
    D: Deserializer<'de>,
{
    #[serde_as]
    #[derive(Deserialize)]
    struct Helper {
        #[serde_as(as = "HeaderBincode")]
        header: Header,
        #[serde_as(as = "Vec<TxEnvelopeBincode<'_, TxEip4844>>")]
        transactions: Vec<TransactionSigned>,
        #[serde_as(as = "Vec<HeaderBincode>")]
        ommers: Vec<Header>,
        withdrawals: Option<Withdrawals>,
    }

    let Helper { header, transactions, ommers, withdrawals } = Helper::deserialize(deserializer)?;
    Ok(Block::new(header, BlockBody { transactions, ommers, withdrawals }))
}

/// The input for the client to execute a block and fully verify the STF (state transition
/// function).
///
/// `parent_state` carries the arena-encoded witness as a raw byte blob, but is `#[serde(skip)]`:
/// it is **never** sent through bincode — the host writes it as a separate SP1 stdin item and the
/// guest reads it directly (zero-copy from the input region) and stuffs it into this field
/// before calling the executor. The guest then decodes the tries by borrowing this blob
/// in-place (see [`ArenaTries::decode`]).
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientExecutorInput<P: NodePrimitives<SignedTx = TransactionSigned>> {
    /// The current block (which will be executed inside the client). Routed through a hand-
    /// rolled bincode-compatible wrapper because alloy v2's `Signed<T>` uses `#[serde(flatten)]`
    /// on the signature field — see [`serialize_block_bincode_compat`] for the full story.
    /// The wrapper is specialized for `Block<TransactionSigned>`, so this struct only derives
    /// `Serialize`/`Deserialize` when `P::SignedTx = TransactionSigned`
    /// (i.e., `P = EthPrimitives`).
    #[serde(
        serialize_with = "serialize_block_bincode_compat",
        deserialize_with = "deserialize_block_bincode_compat"
    )]
    pub current_block: Block<P::SignedTx>,
    /// The previous block headers starting from the most recent. There must be at least one header
    /// to provide the parent state root.
    #[serde_as(as = "Vec<alloy_consensus::serde_bincode_compat::Header>")]
    pub ancestor_headers: Vec<Header>,
    /// Network state as of the parent block, as the arena-codec witness blob. Borrowed
    /// zero-copy in the guest; transmitted as a separate SP1 stdin item (not via bincode).
    #[serde(skip, default)]
    pub parent_state: Vec<u8>,
    /// Account bytecodes.
    pub bytecodes: Vec<Bytecode>,
    /// The genesis block, as a json string.
    pub genesis: Genesis,
    /// The custom beneficiary address.
    pub custom_beneficiary: Option<Address>,
    /// Whether to track the cycle count of opcodes.
    pub opcode_tracking: bool,
}

impl<P: NodePrimitives<SignedTx = TransactionSigned>> ClientExecutorInput<P> {
    /// Gets the immediate parent block's header.
    #[inline(always)]
    pub fn parent_header(&self) -> &Header {
        &self.ancestor_headers[0]
    }

    /// Reverse-chronological sealed headers starting from the current block.
    pub fn sealed_headers(&self) -> impl Iterator<Item = SealedHeader> + '_ {
        once(SealedHeader::seal_slow(self.current_block.header.clone()))
            .chain(self.ancestor_headers.iter().map(|h| SealedHeader::seal_slow(h.clone())))
    }

    /// Decodes the arena witness blob into bump-scoped tries (zero-copy, hash-verifying).
    pub fn tries<'a>(&'a self, bump: &'a Bump) -> Result<ArenaTries<'a>, ClientError> {
        ArenaTries::decode(bump, &self.parent_state).map_err(|_| ClientError::MismatchedStateRoot)
    }

    /// Builds a [`TrieDB`] from decoded tries, verifying state/storage roots, ancestor headers
    /// and bytecodes.
    pub fn witness_db<'a, 'b>(
        &'a self,
        tries: &'a ArenaTries<'b>,
        sealed_headers: &[SealedHeader],
    ) -> Result<TrieDB<'a, 'b>, ClientError> {
        if self.parent_header().state_root() != tries.state_root() {
            return Err(ClientError::MismatchedStateRoot);
        }

        for (hashed_address, storage_trie) in tries.storage_tries.iter() {
            let account =
                tries.state_trie.get_rlp::<TrieAccount>(hashed_address.as_slice()).unwrap();
            let storage_root = account.map_or(EMPTY_ROOT_HASH, |a| a.storage_root);
            if storage_root != storage_trie.hash() {
                return Err(ClientError::MismatchedStorageRoot);
            }
        }

        let bytecode_by_hash =
            self.bytecodes.iter().map(|code| (code.hash_slow(), code)).collect::<HashMap<_, _>>();

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

        Ok(TrieDB { tries, block_hashes, bytecode_by_hash })
    }
}

/// The header committed at the end of execution.
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

/// Witness-backed database revm reads from: arena-encoded tries decoded zero-copy from the
/// input buffer.
///
/// Two lifetimes: `'a` is the (short) borrow of the decoded tries and input, while `'b` is the
/// invariant lifetime of the arena data itself (the bump + witness buffer). Keeping them
/// separate lets the borrow be released after execution so the tries can be mutated for the
/// post-execution state-root update.
#[derive(Debug)]
pub struct TrieDB<'a, 'b> {
    tries: &'a ArenaTries<'b>,
    block_hashes: HashMap<u64, B256>,
    bytecode_by_hash: HashMap<B256, &'a Bytecode>,
}

impl DatabaseRef for TrieDB<'_, '_> {
    type Error = ProviderError;

    fn basic_ref(&self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let hashed_address = keccak256(address);
        let account =
            self.tries.state_trie.get_rlp::<TrieAccount>(hashed_address.as_slice()).unwrap().map(
                |a| AccountInfo {
                    balance: a.balance,
                    nonce: a.nonce,
                    code_hash: a.code_hash,
                    // `account_id` is a runtime-only optimization hint introduced in revm 38;
                    // the guest never replays it across blocks, so leaving it `None` is fine.
                    account_id: None,
                    code: None,
                },
            );
        Ok(account)
    }

    fn code_by_hash_ref(&self, hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(self.bytecode_by_hash.get(&hash).map(|code| (*code).clone()).unwrap())
    }

    fn storage_ref(&self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let hashed_address = keccak256(address);
        let storage_trie = self
            .tries
            .storage_tries
            .get(&hashed_address)
            .expect("A storage trie must be provided for each account");
        Ok(storage_trie
            .get_rlp::<U256>(keccak256(index.to_be_bytes::<32>()).as_slice())
            .expect("Can get from MPT")
            .unwrap_or_default())
    }

    fn block_hash_ref(&self, number: u64) -> Result<B256, Self::Error> {
        Ok(*self
            .block_hashes
            .get(&number)
            .expect("A block hash must be provided for each block number"))
    }
}
