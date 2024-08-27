use reth_primitives::{Address, Bytes, B256};
use reth_trie::Nibbles;

/// Custom database access methods implemented by RSP storage backends.
pub trait ExtDatabaseRef {
    /// The database error type.
    type Error;

    /// Gets the preimage of a trie node given its Keccak hash.
    fn trie_node_ref(&self, hash: B256) -> Result<Bytes, Self::Error>;

    /// Gets the preimage of a trie node given its Keccak hash, with additional context that could
    /// be helpful when the program is not running in a constrained environment.
    fn trie_node_ref_with_context(
        &self,
        hash: B256,
        context: PreimageContext,
    ) -> Result<Bytes, Self::Error>;
}

/// Additional context for retrieving trie node preimages. These are useful when the JSON-RPC node
/// does not serve the `debug_dbGet`.
pub struct PreimageContext<'a> {
    /// The account address if calculating a storage trie root; `None` if calculating the state
    /// root.
    pub address: &'a Option<Address>,
    /// The trie key path of the branch child containing the hash whose preimage is being fetched.
    pub branch_path: &'a Nibbles,
}
