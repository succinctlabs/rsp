use alloy_primitives::hex;

pub(crate) type NodeId = u32;

/// Node data for arena-based trie with zero-copy optimization
#[derive(Clone, Debug, Default, PartialEq, Eq, Ord, PartialOrd)]
pub(crate) enum NodeData<'a> {
    #[default]
    /// Absence of a node. Encoded as empty string in RLP.
    Null,
    /// 16-way branch. Each child is optional; the branch's value slot is unused in our state trie
    /// and must be empty, enforced during decoding.
    Branch([Option<NodeId>; 16]),
    /// Leaf node containing a compact hex-prefix path and a value. Both slices borrow from the
    /// input buffer or bump arena. The path encodes the remainder of the key.
    Leaf(&'a [u8], &'a [u8]),
    /// Extension node containing a compact hex-prefix path and a single child. Path encodes a
    /// shared prefix to skip before continuing at `child`.
    Extension(&'a [u8], NodeId),
    /// Unresolved reference to a node by its Keccak-256 digest (32 bytes). Encountering this in
    /// `get`/`insert`/`delete` is an error; resolution happens in `build_mpt` helpers.
    Digest(&'a [u8]),
}

/// Represents the ways in which one node can reference another node inside the sparse Merkle
/// Patricia Trie (MPT).
///
/// Nodes in the MPT can reference other nodes either directly through their byte representation or
/// indirectly through a hash of their encoding.
#[derive(Copy, Clone, Debug, Hash)]
pub(crate) enum NodeRef<'a> {
    /// Represents a direct reference to another node using its byte encoding. Typically
    /// used for short encodings that are less than 32 bytes in length.
    Bytes(&'a [u8]),
    /// Represents an indirect reference to another node using the Keccak hash of its long
    /// encoding, so its length is always 32. Used for encodings that are not less than 32 bytes in
    /// length.
    Digest(&'a [u8]),
}

impl std::fmt::Display for NodeRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeRef::Bytes(bytes) => write!(f, "Bytes(0x{})", hex::encode(bytes)),
            NodeRef::Digest(digest) => write!(f, "Digest(0x{})", hex::encode(digest)),
        }
    }
}

impl PartialEq for NodeRef<'_> {
    #[inline(always)]
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<'a> NodeRef<'a> {
    #[inline(always)]
    pub(crate) fn as_slice(&self) -> &'a [u8] {
        match self {
            NodeRef::Bytes(slice) => slice,
            NodeRef::Digest(slice) => slice,
        }
    }

    #[inline(always)]
    pub(crate) fn from_rlp_slice(slice: &'a [u8]) -> Self {
        if slice.len() == 33 {
            Self::Digest(&slice[1..])
        } else {
            debug_assert!(slice.len() < 32);
            Self::Bytes(slice)
        }
    }
}
