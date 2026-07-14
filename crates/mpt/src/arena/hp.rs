//! Hex-prefix (HP) helpers and nibble utilities for MPT paths.
use core::{cmp, iter};
use smallvec::SmallVec;

/// Compact vector for nibble sequences used in key traversal.
pub(crate) type Nibbles = SmallVec<[u8; 64]>;

// Hex-prefix (HP) encoding flags for MPT paths
pub(crate) const HP_FLAG_ODD: u8 = 0x10; // path has odd number of nibbles; low nibble of first byte is data
#[allow(dead_code)]
pub(crate) const HP_FLAG_LEAF: u8 = 0x20; // node is a leaf (vs extension)

/// Returns the length of the common prefix (in nibbles) between two nibble slices.
#[inline]
pub(crate) fn lcp(a: &[u8], b: &[u8]) -> usize {
    for (i, (a, b)) in iter::zip(a, b).enumerate() {
        if a != b {
            return i;
        }
    }
    cmp::min(a.len(), b.len())
}

/// Converts a byte slice into a vector of nibbles.
/// Uses `SmallVec` to avoid heap allocation for typical key sizes (≤32 bytes = 64 nibbles).
#[inline]
pub(crate) fn to_nibs(slice: &[u8]) -> Nibbles {
    let mut result = SmallVec::with_capacity(2 * slice.len());
    for byte in slice {
        result.push(byte >> 4);
        result.push(byte & 0x0f);
    }
    result
}

/// Decodes a compact hex-prefix-encoded path (as used in MPT leaf/extension nodes)
/// into its nibble sequence. This allocates a `SmallVec` with the exact nibble capacity.
#[inline]
pub(crate) fn prefix_to_nibs(encoded_path: &[u8]) -> Nibbles {
    if encoded_path.is_empty() {
        return SmallVec::new();
    }

    let first_byte = encoded_path[0];
    let is_odd = (first_byte & HP_FLAG_ODD) != 0;
    // Nibble count: if odd, first byte contains 1 nibble of data; otherwise, first byte
    // contains only flags. Remaining bytes always contain two nibbles each.
    let nib_count = 2 * (encoded_path.len() - 1) + if is_odd { 1 } else { 0 };
    let mut nibs = SmallVec::with_capacity(nib_count);

    // Handle the first nibble if odd length
    if is_odd {
        nibs.push(first_byte & 0x0f);
    }

    // Process remaining bytes, starting from index 1
    for &byte in &encoded_path[1..] {
        nibs.push(byte >> 4); // High nibble
        nibs.push(byte & 0x0f); // Low nibble
    }

    nibs
}

/// Returns the number of nibbles encoded in a compact hex-prefix path.
#[inline]
pub(crate) fn encoded_path_nibble_count(encoded_path: &[u8]) -> usize {
    if encoded_path.is_empty() {
        return 0;
    }
    let is_odd = (encoded_path[0] & HP_FLAG_ODD) != 0;
    2 * (encoded_path.len() - 1) + if is_odd { 1 } else { 0 }
}

/// Compares a compact hex-prefix path with a nibble slice for equality without allocating.
#[inline]
pub(crate) fn encoded_path_eq_nibs(encoded_path: &[u8], nibs: &[u8]) -> bool {
    let nib_count = encoded_path_nibble_count(encoded_path);
    if nib_count != nibs.len() {
        return false;
    }
    if nib_count == 0 {
        return true;
    }

    let first = encoded_path[0];
    let is_odd = (first & HP_FLAG_ODD) != 0;
    let mut i = 0usize; // index in nibs
    let mut j = 1usize; // index in encoded_path bytes

    if is_odd {
        if nibs[i] != (first & 0x0f) {
            return false;
        }
        i += 1;
    }

    while i + 1 < nibs.len() {
        let b = encoded_path[j];
        if nibs[i] != (b >> 4) {
            return false;
        }
        if nibs[i + 1] != (b & 0x0f) {
            return false;
        }
        i += 2;
        j += 1;
    }

    if i < nibs.len() {
        // one last high nibble remains
        let b = encoded_path[j];
        if nibs[i] != (b >> 4) {
            return false;
        }
    }
    true
}

/// If `encoded_path` is a prefix of `nibs`, returns the tail `&nibs[matched_len..]`.
#[inline]
pub(crate) fn encoded_path_strip_prefix<'a>(
    encoded_path: &[u8],
    nibs: &'a [u8],
) -> Option<&'a [u8]> {
    let nib_count = encoded_path_nibble_count(encoded_path);
    if nib_count > nibs.len() {
        return None;
    }
    if nib_count == 0 {
        return Some(nibs);
    }

    let first = encoded_path[0];
    let is_odd = (first & HP_FLAG_ODD) != 0;
    let mut i = 0usize; // index in nibs
    let mut j = 1usize; // index in encoded_path bytes

    if is_odd {
        if nibs[i] != (first & 0x0f) {
            return None;
        }
        i += 1;
    }

    while i + 1 < nib_count {
        let b = encoded_path[j];
        if nibs[i] != (b >> 4) {
            return None;
        }
        if nibs[i + 1] != (b & 0x0f) {
            return None;
        }
        i += 2;
        j += 1;
    }

    if i < nib_count {
        let b = encoded_path[j];
        if nibs[i] != (b >> 4) {
            return None;
        }
        i += 1;
    }
    Some(&nibs[i..])
}

/// Encodes nibbles into the standard hex-prefix format directly into the bump arena.
#[inline]
pub(crate) fn to_encoded_path_with_bump<'a>(
    bump: &'a bumpalo::Bump,
    nibs: &[u8],
    is_leaf: bool,
) -> &'a [u8] {
    let is_odd = !nibs.len().is_multiple_of(2);
    let encoded_len = 1 + (nibs.len() / 2);
    let mut encoded = bumpalo::collections::Vec::with_capacity_in(encoded_len, bump);

    let mut prefix = if is_leaf { 0x20 } else { 0x00 };
    if is_odd {
        prefix |= 0x10;
        encoded.push(prefix | nibs[0]);
        for i in (1..nibs.len()).step_by(2) {
            encoded.push((nibs[i] << 4) | nibs[i + 1]);
        }
    } else {
        encoded.push(prefix);
        for i in (0..nibs.len()).step_by(2) {
            encoded.push((nibs[i] << 4) | nibs[i + 1]);
        }
    }

    encoded.into_bump_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoded_path_nibble_count() {
        assert_eq!(encoded_path_nibble_count(&[]), 0);
        // ODD+LEAF with one nibble 0xA
        assert_eq!(encoded_path_nibble_count(&[HP_FLAG_ODD | HP_FLAG_LEAF | 0x0a]), 1);
        // EVEN+EXT with 2 bytes => 4 nibbles
        assert_eq!(encoded_path_nibble_count(&[0x00, 0xab, 0xcd]), 4);
    }

    #[test]
    fn test_eq_and_strip_prefix() {
        // path [1, 2, 3] as HP: ODD + EXT, first byte 0x10 | 0x1, then 0x23
        let path = [HP_FLAG_ODD | 0x01, 0x23];
        let key = [1, 2, 3];
        assert!(encoded_path_eq_nibs(&path, &key));
        assert_eq!(encoded_path_strip_prefix(&path, &key), Some(&[][..]));

        let key_longer = [1, 2, 3, 4, 5];
        assert_eq!(encoded_path_strip_prefix(&path, &key_longer), Some(&key_longer[3..]));

        let key_mismatch = [1, 2, 4];
        assert!(encoded_path_strip_prefix(&path, &key_mismatch).is_none());
    }

    #[test]
    fn test_to_encoded_path() {
        let bump = bumpalo::Bump::new();

        // extension node with an even path length
        let nibbles = vec![0x0a, 0x0b, 0x0c, 0x0d];
        assert_eq!(to_encoded_path_with_bump(&bump, &nibbles, false), vec![0x00, 0xab, 0xcd]);
        // extension node with an odd path length
        let nibbles = vec![0x0a, 0x0b, 0x0c];
        assert_eq!(to_encoded_path_with_bump(&bump, &nibbles, false), vec![0x1a, 0xbc]);
        // leaf node with an even path length
        let nibbles = vec![0x0a, 0x0b, 0x0c, 0x0d];
        assert_eq!(to_encoded_path_with_bump(&bump, &nibbles, true), vec![0x20, 0xab, 0xcd]);
        // leaf node with an odd path length
        let nibbles = vec![0x0a, 0x0b, 0x0c];
        assert_eq!(to_encoded_path_with_bump(&bump, &nibbles, true), vec![0x3a, 0xbc]);
    }

    #[test]
    fn test_lcp() {
        let cases = [
            (vec![], vec![], 0),
            (vec![0xa], vec![0xa], 1),
            (vec![0xa, 0xb], vec![0xa, 0xc], 1),
            (vec![0xa, 0xb], vec![0xa, 0xb], 2),
            (vec![0xa, 0xb], vec![0xa, 0xb, 0xc], 2),
            (vec![0xa, 0xb, 0xc], vec![0xa, 0xb, 0xc], 3),
            (vec![0xa, 0xb, 0xc], vec![0xa, 0xb, 0xc, 0xd], 3),
            (vec![0xa, 0xb, 0xc, 0xd], vec![0xa, 0xb, 0xc, 0xd], 4),
        ];
        for (a, b, cpl) in cases {
            assert_eq!(lcp(&a, &b), cpl)
        }
    }
}
