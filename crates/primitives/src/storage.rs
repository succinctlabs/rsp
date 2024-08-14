use reth_primitives::{Bytes, B256};

/// Custom database access methods implemented by RSP storage backends.
pub trait ExtDatabaseRef {
    /// The database error type.
    type Error;

    /// Gets the preimage of a trie node given its Keccak hash.
    fn trie_node_ref(&self, hash: B256) -> Result<Bytes, Self::Error>;
}
