use alloy_primitives::{b256, keccak256, B256};

use super::{node::NodeData, Error, Mpt};

/// The legacy `MptNode` and the arena `Mpt` must agree on the root hash after a structural
/// conversion, and the encode -> decode round-trip (the host->guest witness path) must preserve
/// it. This is the correctness foundation for the arena witness integration.
#[test]
fn test_from_mpt_node_roundtrip() {
    use bumpalo::Bump;

    use crate::mpt::MptNode;

    let mut legacy = MptNode::default();
    for i in 0..2000u64 {
        legacy.insert_rlp(keccak256(i.to_be_bytes()).as_slice(), i).unwrap();
    }
    let legacy_root = legacy.hash();

    // Structural conversion must reproduce the exact root.
    let bump = Bump::new();
    let arena = Mpt::from_mpt_node(&bump, &legacy);
    assert_eq!(arena.hash(), legacy_root, "arena conversion root mismatch");

    // encode_trie (host) -> decode_trie (guest, zero-copy + hash-verifying) must round-trip.
    let encoded = arena.encode_trie();
    let num_nodes = arena.num_nodes();
    let decode_bump = Bump::new();
    let mut slice = encoded.as_slice();
    let decoded = Mpt::decode_trie(&decode_bump, &mut slice, num_nodes).unwrap();
    assert_eq!(decoded.hash(), legacy_root, "decoded arena root mismatch");
}

trait RlpBytes {
    /// Returns the RLP-encoding.
    fn to_rlp(&self) -> Vec<u8>;
}

impl<T> RlpBytes for T
where
    T: alloy_rlp::Encodable,
{
    #[inline]
    fn to_rlp(&self) -> Vec<u8> {
        let rlp_length = self.length();
        let mut out = Vec::with_capacity(rlp_length);
        self.encode(&mut out);
        debug_assert_eq!(out.len(), rlp_length);
        out
    }
}

#[test]
fn test_empty() {
    let bump = bumpalo::Bump::new();
    let trie = Mpt::new(&bump);

    assert!(trie.is_empty());
    let expected = b256!("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421");
    assert_eq!(expected, trie.hash());
}

#[test]
fn test_empty_key() -> Result<(), Error> {
    let bump = bumpalo::Bump::new();
    let mut trie = Mpt::new(&bump);

    trie.insert(&[], b"empty")?;
    assert_eq!(trie.get(&[])?, Some(b"empty".as_ref()));
    assert!(trie.delete(&[])?);

    Ok(())
}

#[test]
fn test_branch_value() {
    let bump = bumpalo::Bump::new();
    let mut trie = Mpt::new(&bump);
    trie.insert(b"do", b"verb").unwrap();
    // leads to a branch with value which is not supported
    trie.insert(b"dog", b"puppy").unwrap_err();
}

#[test]
fn test_insert() -> Result<(), Error> {
    let bump = bumpalo::Bump::new();
    let mut trie = Mpt::new(&bump);

    let key_vals = vec![
        ("painting", "place"),
        ("guest", "ship"),
        ("mud", "leave"),
        ("paper", "call"),
        ("gate", "boast"),
        ("tongue", "gain"),
        ("baseball", "wait"),
        ("tale", "lie"),
        ("mood", "cope"),
        ("menu", "fear"),
    ];
    for (key, val) in &key_vals {
        assert!(trie.insert(key.as_bytes(), val.as_bytes())?);
    }

    // Identical root hash to the legacy `MptNode` implementation's `test_insert`.
    let expected = b256!("2bab6cdf91a23ebf3af683728ea02403a98346f99ed668eec572d55c70a4b08f");
    assert_eq!(expected, trie.hash());

    for (key, value) in &key_vals {
        let retrieved = trie.get(key.as_bytes())?.unwrap();
        assert_eq!(retrieved, value.as_bytes());
    }

    // check inserting duplicate keys
    assert!(trie.insert(key_vals[0].0.as_bytes(), b"new")?);
    assert!(!trie.insert(key_vals[0].0.as_bytes(), b"new")?);

    Ok(())
}

#[test]
fn test_keccak_trie() -> Result<(), Error> {
    const N: usize = 512;

    let bump = bumpalo::Bump::new();
    let mut trie = Mpt::new(&bump);

    for i in 0..N {
        assert!(trie.insert_rlp(keccak256(i.to_be_bytes()).as_slice(), i)?);

        // check hash against trie built in reverse
        let bump2 = bumpalo::Bump::new();
        let mut trie2 = Mpt::new(&bump2);
        for j in (0..=i).rev() {
            trie2.insert_rlp(keccak256(j.to_be_bytes()).as_slice(), j)?;
        }
        assert_eq!(trie.hash(), trie2.hash());
    }

    // Identical root hash to the legacy `MptNode` implementation's `test_keccak_trie`.
    let expected = b256!("7310027edebdd1f7c950a7fb3413d551e85dff150d45aca4198c2f6315f9b4a7");
    assert_eq!(trie.hash(), expected);

    for i in 0..N {
        assert_eq!(trie.get_rlp(keccak256(i.to_be_bytes()).as_slice())?, Some(i));
        assert!(trie.get(keccak256((i + N).to_be_bytes()).as_slice())?.is_none());
    }

    for i in 0..N {
        assert!(trie.delete(keccak256(i.to_be_bytes()).as_slice())?);

        let bump2 = bumpalo::Bump::new();
        let mut trie2 = Mpt::new(&bump2);
        for j in ((i + 1)..N).rev() {
            trie2.insert_rlp(keccak256(j.to_be_bytes()).as_slice(), j)?;
        }
        assert_eq!(trie.hash(), trie2.hash());
    }
    assert!(trie.is_empty());

    Ok(())
}

#[test]
fn test_index_trie() -> Result<(), Error> {
    const N: usize = 512;

    let bump = bumpalo::Bump::new();
    let mut trie = Mpt::new(&bump);

    for i in 0..N {
        assert!(trie.insert_rlp(&i.to_rlp(), i)?);
    }

    for i in 0..N {
        assert_eq!(trie.get_rlp(&i.to_rlp())?, Some(i));
        assert!(trie.get(&(i + N).to_rlp())?.is_none());
    }

    for i in 0..N {
        assert!(trie.delete(&i.to_rlp()).unwrap());
    }
    assert!(trie.is_empty());

    Ok(())
}

#[test]
fn test_encode_decode_roundtrip() -> Result<(), Error> {
    const N: usize = 512;

    let bump = bumpalo::Bump::new();
    let mut trie = Mpt::new(&bump);

    for i in 0..N {
        assert!(trie.insert_rlp(keccak256(i.to_be_bytes()).as_slice(), i)?);
    }

    let root_hash = trie.hash();
    let encoded = trie.encode_trie();

    let bump2 = bumpalo::Bump::new();
    let recovered = Mpt::decode_trie(&bump2, &mut encoded.as_slice(), trie.num_nodes())?;
    assert_eq!(recovered.hash(), root_hash);

    for i in 0..N {
        assert_eq!(recovered.get_rlp(keccak256(i.to_be_bytes()).as_slice())?, Some(i));
    }

    Ok(())
}

/// A `delete` that collapses a branch onto an unresolved `Digest` sibling must error rather
/// than silently producing a wrong root. (Contrast with the legacy `MptNode`, which wraps the
/// digest in an `Extension`.)
#[test]
fn test_delete_with_unresolved_sibling_errors() {
    use super::hp::to_encoded_path_with_bump;

    let bump = bumpalo::Bump::new();
    let mut trie = Mpt::new(&bump);

    let fake_digest: &[u8] = bump.alloc_slice_copy(&[0xABu8; 32]);

    let leaf_path = to_encoded_path_with_bump(&bump, &[0], true);
    let leaf_id = trie.add_node(NodeData::Leaf(leaf_path, b"value"), None);

    let digest_id = trie.add_node(NodeData::Digest(fake_digest), None);

    let mut children: [Option<u32>; 16] = Default::default();
    children[0] = Some(leaf_id);
    children[1] = Some(digest_id);
    let branch_id = trie.add_node(NodeData::Branch(children), None);

    trie.set_root_id(branch_id);

    match trie.delete(&[0x00]) {
        Err(Error::NodeNotResolved(hash)) => {
            assert_eq!(hash, B256::from_slice(fake_digest));
        }
        Ok(_) => panic!("Expected NodeNotResolved error, but delete succeeded"),
        Err(e) => panic!("Expected NodeNotResolved error, got: {e:?}"),
    }
}
