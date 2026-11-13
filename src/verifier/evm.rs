// SPDX-License-Identifier: MIT OR Apache-2.0
//! EVM Merkle Patricia Trie proof verification (DESIGN.md §5.3, optimistic tier).
//!
//! Verifies an inclusion or exclusion proof against a trie root, such as a receipts root or
//! state root taken from a block header. The header itself is not checked here. Trusting its
//! root requires the light-client sync, which is not implemented yet.
//!
//! Scope: nodes are checked by hash against the root, and the RLP and compact-path encodings
//! are checked for well-formedness. Non-canonical RLP is rejected.

use std::collections::HashMap;

use sha3::{Digest, Keccak256};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MptError {
    #[error("malformed RLP encoding")]
    MalformedRlp,
    #[error("malformed trie node")]
    MalformedNode,
    #[error("no proof node hashes to the referenced root or child")]
    MissingNode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Rlp {
    Bytes(Vec<u8>),
    List(Vec<Rlp>),
}

fn slice(input: &[u8], start: usize, len: usize) -> Result<&[u8], MptError> {
    let end = start.checked_add(len).ok_or(MptError::MalformedRlp)?;
    input.get(start..end).ok_or(MptError::MalformedRlp)
}

/// Big-endian length field. Must be minimal and at most 8 bytes.
fn read_len(bytes: &[u8]) -> Result<usize, MptError> {
    if bytes.is_empty() || bytes.len() > 8 || bytes[0] == 0 {
        return Err(MptError::MalformedRlp);
    }
    Ok(bytes
        .iter()
        .fold(0usize, |acc, &b| (acc << 8) | usize::from(b)))
}

fn decode_item(input: &[u8]) -> Result<(Rlp, &[u8]), MptError> {
    let &first = input.first().ok_or(MptError::MalformedRlp)?;
    match first {
        0x00..=0x7f => Ok((Rlp::Bytes(vec![first]), &input[1..])),
        0x80..=0xb7 => {
            let len = usize::from(first - 0x80);
            let body = slice(input, 1, len)?;
            if len == 1 && body[0] < 0x80 {
                return Err(MptError::MalformedRlp); // should have been a single byte
            }
            Ok((Rlp::Bytes(body.to_vec()), &input[1 + len..]))
        }
        0xb8..=0xbf => {
            let lenlen = usize::from(first - 0xb7);
            let len = read_len(slice(input, 1, lenlen)?)?;
            if len <= 55 {
                return Err(MptError::MalformedRlp); // short form must be used
            }
            let body = slice(input, 1 + lenlen, len)?;
            Ok((Rlp::Bytes(body.to_vec()), &input[1 + lenlen + len..]))
        }
        0xc0..=0xf7 => {
            let len = usize::from(first - 0xc0);
            let payload = slice(input, 1, len)?;
            Ok((Rlp::List(decode_list_payload(payload)?), &input[1 + len..]))
        }
        _ => {
            let lenlen = usize::from(first - 0xf7);
            let len = read_len(slice(input, 1, lenlen)?)?;
            if len <= 55 {
                return Err(MptError::MalformedRlp);
            }
            let payload = slice(input, 1 + lenlen, len)?;
            Ok((
                Rlp::List(decode_list_payload(payload)?),
                &input[1 + lenlen + len..],
            ))
        }
    }
}

fn decode_list_payload(mut payload: &[u8]) -> Result<Vec<Rlp>, MptError> {
    let mut items = Vec::new();
    while !payload.is_empty() {
        let (item, rest) = decode_item(payload)?;
        items.push(item);
        payload = rest;
    }
    Ok(items)
}

fn decode_rlp(input: &[u8]) -> Result<Rlp, MptError> {
    let (item, rest) = decode_item(input)?;
    if rest.is_empty() {
        Ok(item)
    } else {
        Err(MptError::MalformedRlp)
    }
}

/// Hex-prefix ("compact") path decoding. Returns the nibbles and whether the node is a leaf.
fn compact_decode(bytes: &[u8]) -> Result<(Vec<u8>, bool), MptError> {
    let &first = bytes.first().ok_or(MptError::MalformedNode)?;
    let flags = first >> 4;
    if flags > 3 {
        return Err(MptError::MalformedNode);
    }
    let is_leaf = flags & 2 != 0;
    let odd = flags & 1 != 0;
    let mut nibbles = Vec::with_capacity(bytes.len() * 2);
    if odd {
        nibbles.push(first & 0x0f);
    }
    for &b in &bytes[1..] {
        nibbles.push(b >> 4);
        nibbles.push(b & 0x0f);
    }
    Ok((nibbles, is_leaf))
}

fn to_nibbles(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().flat_map(|b| [b >> 4, b & 0x0f]).collect()
}

fn keccak(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

type Nodes<'a> = HashMap<[u8; 32], &'a [u8]>;

fn bytes_of(item: &Rlp) -> Result<&[u8], MptError> {
    match item {
        Rlp::Bytes(b) => Ok(b),
        Rlp::List(_) => Err(MptError::MalformedNode),
    }
}

fn walk(node: &Rlp, key: &[u8], nodes: &Nodes) -> Result<Option<Vec<u8>>, MptError> {
    let Rlp::List(items) = node else {
        return Err(MptError::MalformedNode);
    };
    match items.len() {
        17 => {
            // Branch node. A key that ends here is stored in the 17th slot.
            if key.is_empty() {
                let value = bytes_of(&items[16])?;
                return Ok((!value.is_empty()).then(|| value.to_vec()));
            }
            walk_child(&items[usize::from(key[0])], &key[1..], nodes)
        }
        2 => {
            let (path, is_leaf) = compact_decode(bytes_of(&items[0])?)?;
            if key.len() < path.len() || key[..path.len()] != path[..] {
                return Ok(None);
            }
            let rest = &key[path.len()..];
            if is_leaf {
                if rest.is_empty() {
                    Ok(Some(bytes_of(&items[1])?.to_vec()))
                } else {
                    Ok(None)
                }
            } else {
                walk_child(&items[1], rest, nodes)
            }
        }
        _ => Err(MptError::MalformedNode),
    }
}

fn walk_child(child: &Rlp, key: &[u8], nodes: &Nodes) -> Result<Option<Vec<u8>>, MptError> {
    match child {
        Rlp::Bytes(b) if b.is_empty() => Ok(None),
        Rlp::Bytes(h) if h.len() == 32 => {
            let hash: [u8; 32] = h.as_slice().try_into().expect("length checked");
            let raw = nodes.get(&hash).ok_or(MptError::MissingNode)?;
            walk(&decode_rlp(raw)?, key, nodes)
        }
        // Nodes shorter than 32 bytes are embedded inline rather than referenced by hash.
        Rlp::List(_) => walk(child, key, nodes),
        Rlp::Bytes(_) => Err(MptError::MalformedNode),
    }
}

/// Verify a Merkle Patricia Trie proof.
///
/// `key` is the raw trie key, for example keccak(address) for the state trie. `proof` holds the
/// RLP-encoded nodes along the path, root first. Returns `Some(value)` for inclusion and `None`
/// for proven absence.
pub fn verify_proof(
    root: [u8; 32],
    key: &[u8],
    proof: &[Vec<u8>],
) -> Result<Option<Vec<u8>>, MptError> {
    let nodes: Nodes = proof.iter().map(|n| (keccak(n), n.as_slice())).collect();
    let root_raw = nodes.get(&root).ok_or(MptError::MissingNode)?;
    walk(&decode_rlp(root_raw)?, &to_nibbles(key), &nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_bytes(b: &[u8]) -> Vec<u8> {
        if b.len() == 1 && b[0] < 0x80 {
            return b.to_vec();
        }
        if b.len() <= 55 {
            let mut out = vec![0x80 + b.len() as u8];
            out.extend_from_slice(b);
            return out;
        }
        let len = b.len().to_be_bytes();
        let lenlen = len.iter().skip_while(|&&x| x == 0).count();
        let mut out = vec![0xb7 + lenlen as u8];
        out.extend_from_slice(&len[8 - lenlen..]);
        out.extend_from_slice(b);
        out
    }

    fn encode_list(items: &[Vec<u8>]) -> Vec<u8> {
        let payload: Vec<u8> = items.concat();
        if payload.len() <= 55 {
            let mut out = vec![0xc0 + payload.len() as u8];
            out.extend(payload);
            return out;
        }
        let len = payload.len().to_be_bytes();
        let lenlen = len.iter().skip_while(|&&x| x == 0).count();
        let mut out = vec![0xf7 + lenlen as u8];
        out.extend_from_slice(&len[8 - lenlen..]);
        out.extend(payload);
        out
    }

    /// Hex-prefix encode a nibble path.
    fn compact_encode(nibbles: &[u8], is_leaf: bool) -> Vec<u8> {
        let flag = if is_leaf { 2u8 } else { 0 };
        let mut out = Vec::new();
        let mut rest = nibbles;
        if nibbles.len() % 2 == 1 {
            out.push(((flag | 1) << 4) | nibbles[0]);
            rest = &nibbles[1..];
        } else {
            out.push(flag << 4);
        }
        for pair in rest.chunks(2) {
            out.push((pair[0] << 4) | pair[1]);
        }
        out
    }

    fn leaf(path_nibbles: &[u8], value: &[u8]) -> Vec<u8> {
        encode_list(&[
            encode_bytes(&compact_encode(path_nibbles, true)),
            encode_bytes(value),
        ])
    }

    fn key_hash(raw: &[u8]) -> [u8; 32] {
        keccak(raw)
    }

    const VALUE: &[u8] = &[0xabu8; 40]; // long enough that leaves are hashed, not inlined

    #[test]
    fn single_leaf_trie_includes_and_excludes() {
        let h = key_hash(b"alice");
        let node = leaf(&to_nibbles(&h), VALUE);
        let root = keccak(&node);

        assert_eq!(
            verify_proof(root, &h, std::slice::from_ref(&node)),
            Ok(Some(VALUE.to_vec()))
        );
        // A different key shares the root but has a different path: proven absent.
        let other = key_hash(b"bob");
        assert_eq!(verify_proof(root, &other, &[node]), Ok(None));
    }

    #[test]
    fn tampered_node_is_missing_from_the_root() {
        let h = key_hash(b"alice");
        let node = leaf(&to_nibbles(&h), VALUE);
        let root = keccak(&node);
        let mut bad = node.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert_eq!(verify_proof(root, &h, &[bad]), Err(MptError::MissingNode));
    }

    #[test]
    fn two_leaf_branch_trie() {
        let h1 = key_hash(b"alice");
        let mut h2 = key_hash(b"bob");
        // Force different first nibbles so the root is a branch node.
        let n1 = to_nibbles(&h1)[0];
        let n2 = (n1 + 1) % 16;
        let mut nib2 = to_nibbles(&h2);
        nib2[0] = n2;
        h2 = nib2
            .chunks(2)
            .map(|p| (p[0] << 4) | p[1])
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();

        let leaf1 = leaf(&to_nibbles(&h1)[1..], VALUE);
        let leaf2 = leaf(
            &to_nibbles(&h2)[1..],
            b"second value, also long enough to hash ...",
        );
        let mut slots: Vec<Vec<u8>> = vec![encode_bytes(&[]); 17];
        slots[usize::from(n1)] = encode_bytes(&keccak(&leaf1));
        slots[usize::from(n2)] = encode_bytes(&keccak(&leaf2));
        let branch = encode_list(&slots);
        let root = keccak(&branch);

        assert_eq!(
            verify_proof(root, &h1, &[branch.clone(), leaf1.clone()]),
            Ok(Some(VALUE.to_vec()))
        );
        assert_eq!(
            verify_proof(root, &h2, &[branch.clone(), leaf2.clone()]),
            Ok(Some(b"second value, also long enough to hash ...".to_vec()))
        );
        // A third first nibble, so its branch slot is empty and the key is proven absent.
        let n3 = (n1 + 3) % 16;
        let mut nib3 = to_nibbles(&h1);
        nib3[0] = n3;
        let h3: Vec<u8> = nib3.chunks(2).map(|p| (p[0] << 4) | p[1]).collect();
        assert_eq!(verify_proof(root, &h3, &[branch]), Ok(None));
    }

    #[test]
    fn rlp_rejects_non_canonical_encodings() {
        // Single byte below 0x80 encoded with a length prefix.
        assert_eq!(decode_rlp(&[0x81, 0x05]), Err(MptError::MalformedRlp));
        // Trailing bytes after a complete item.
        assert_eq!(decode_rlp(&[0x01, 0x02]), Err(MptError::MalformedRlp));
        // Long-form length used for a short payload.
        assert_eq!(
            decode_rlp(&[0xb8, 0x02, 0xaa, 0xbb]),
            Err(MptError::MalformedRlp)
        );
    }

    #[test]
    fn compact_decoding_matches_encoding() {
        let nibbles = [1u8, 2, 3];
        let (decoded, is_leaf) = compact_decode(&compact_encode(&nibbles, true)).unwrap();
        assert_eq!(decoded, nibbles.to_vec());
        assert!(is_leaf);
        let (decoded, is_leaf) = compact_decode(&compact_encode(&[0xa, 0xb], false)).unwrap();
        assert_eq!(decoded, vec![0xa, 0xb]);
        assert!(!is_leaf);
    }
}
