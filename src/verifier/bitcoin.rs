// SPDX-License-Identifier: MIT OR Apache-2.0
//! Bitcoin SPV proof verification (DESIGN.md §5.3, slow-finality tier).
//!
//! Checks a transaction's inclusion in a header chain. It checks each header's proof of work
//! against its own `nBits`, the link between consecutive headers, the merkle branch from the
//! txid to the block's merkle root, and the number of confirmations.
//!
//! Limitations, stated so they are not mistaken for full validation:
//! - Difficulty retargeting is not checked. Each header is only tested against the target its
//!   own `nBits` claims. The chain's starting point must be trusted, for example a
//!   checkpoint in the configuration.
//! - Byte order follows Bitcoin's conventions. Hashes are stored in internal (little-endian)
//!   order, and `txid` and the merkle branch must be given that way. Display hashes are the
//!   reverse of internal order.

use sha2::{Digest, Sha256};
use thiserror::Error;

pub const HEADER_LEN: usize = 80;
const MERKLE_ROOT_OFFSET: usize = 36;
const BITS_OFFSET: usize = 72;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SpvError {
    #[error("header chain is empty")]
    EmptyChain,
    #[error("block index {0} is outside the header chain")]
    BlockIndexOutOfRange(usize),
    #[error("header {0} does not meet its claimed proof-of-work target")]
    ProofOfWorkFailed(usize),
    #[error("header {0} does not reference its predecessor")]
    BrokenLinkage(usize),
    #[error("merkle branch does not reach the block's merkle root")]
    MerkleMismatch,
    #[error("transaction has {have} confirmations, {need} required")]
    InsufficientConfirmations { have: usize, need: usize },
}

pub fn double_sha256(data: &[u8]) -> [u8; 32] {
    let first = Sha256::digest(data);
    Sha256::digest(first).into()
}

/// Expand compact `nBits` into a 256-bit big-endian target. Returns `None` for negative or
/// overflowing encodings, which are never valid.
pub fn target_from_bits(bits: u32) -> Option<[u8; 32]> {
    if bits & 0x0080_0000 != 0 {
        return None;
    }
    let exponent = (bits >> 24) as i64;
    let mantissa = bits & 0x007f_ffff;
    let mantissa_bytes = [
        (mantissa >> 16) as u8,
        (mantissa >> 8) as u8,
        mantissa as u8,
    ];
    let mut target = [0u8; 32];
    for (k, &byte) in mantissa_bytes.iter().enumerate() {
        // Significance of this byte, counted from the least significant end.
        let lsb_pos = exponent - 3 + (2 - k as i64);
        if lsb_pos < 0 {
            continue;
        }
        if lsb_pos >= 32 {
            if byte != 0 {
                return None;
            }
            continue;
        }
        target[31 - lsb_pos as usize] = byte;
    }
    Some(target)
}

fn header_hash_internal(header: &[u8; HEADER_LEN]) -> [u8; 32] {
    double_sha256(header)
}

/// Whether the header's hash, read as a big-endian number, is at or below its own target.
pub fn meets_own_target(header: &[u8; HEADER_LEN]) -> bool {
    let bits = u32::from_le_bytes(
        header[BITS_OFFSET..BITS_OFFSET + 4]
            .try_into()
            .expect("slice has length 4"),
    );
    let Some(target) = target_from_bits(bits) else {
        return false;
    };
    let mut hash = header_hash_internal(header);
    hash.reverse();
    hash <= target
}

/// Verify a merkle branch. `index` gives the leaf's position, and its bits pick the side of
/// each sibling. Bits beyond the branch length must be zero.
pub fn merkle_branch_reaches_root(
    txid: [u8; 32],
    branch: &[[u8; 32]],
    index: u32,
    root: [u8; 32],
) -> bool {
    if branch.len() < 32 && (index >> branch.len()) != 0 {
        return false;
    }
    let mut node = txid;
    let mut idx = index;
    for sibling in branch {
        let mut buf = [0u8; 64];
        if idx & 1 == 0 {
            buf[..32].copy_from_slice(&node);
            buf[32..].copy_from_slice(sibling);
        } else {
            buf[..32].copy_from_slice(sibling);
            buf[32..].copy_from_slice(&node);
        }
        node = double_sha256(&buf);
        idx >>= 1;
    }
    node == root
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpvProof {
    /// Consecutive headers, oldest first. The block holding the transaction is at `block_index`.
    pub headers: Vec<[u8; HEADER_LEN]>,
    pub block_index: usize,
    pub txid: [u8; 32],
    pub merkle_branch: Vec<[u8; 32]>,
    pub tx_index: u32,
}

/// Verify an SPV proof. Returns the confirmation count, which includes the transaction's own block.
pub fn verify_spv(proof: &SpvProof, min_confirmations: usize) -> Result<usize, SpvError> {
    if proof.headers.is_empty() {
        return Err(SpvError::EmptyChain);
    }
    if proof.block_index >= proof.headers.len() {
        return Err(SpvError::BlockIndexOutOfRange(proof.block_index));
    }
    for (i, header) in proof.headers.iter().enumerate() {
        if !meets_own_target(header) {
            return Err(SpvError::ProofOfWorkFailed(i));
        }
        if i > 0 {
            let prev_hash = header_hash_internal(&proof.headers[i - 1]);
            if header[..32] != prev_hash {
                return Err(SpvError::BrokenLinkage(i));
            }
        }
    }

    let block = &proof.headers[proof.block_index];
    let root: [u8; 32] = block[MERKLE_ROOT_OFFSET..MERKLE_ROOT_OFFSET + 32]
        .try_into()
        .expect("slice has length 32");
    if !merkle_branch_reaches_root(proof.txid, &proof.merkle_branch, proof.tx_index, root) {
        return Err(SpvError::MerkleMismatch);
    }

    let confirmations = proof.headers.len() - proof.block_index;
    if confirmations < min_confirmations {
        return Err(SpvError::InsufficientConfirmations {
            have: confirmations,
            need: min_confirmations,
        });
    }
    Ok(confirmations)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bitcoin mainnet genesis block header.
    const GENESIS_HEADER_HEX: &str = "0100000000000000000000000000000000000000000000000000000000000000000000003ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a29ab5f49ffff001d1dac2b7c";
    /// Display (big-endian) form of the genesis block hash.
    const GENESIS_HASH_DISPLAY: &str =
        "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f";
    /// Display form of the genesis coinbase txid, which is also the merkle root.
    const GENESIS_TXID_DISPLAY: &str =
        "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";

    fn hex_to_bytes(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    fn reversed(s: &str) -> [u8; 32] {
        let mut b: [u8; 32] = hex_to_bytes(s).try_into().unwrap();
        b.reverse();
        b
    }

    fn genesis() -> [u8; HEADER_LEN] {
        hex_to_bytes(GENESIS_HEADER_HEX).try_into().unwrap()
    }

    #[test]
    fn genesis_hash_matches_known_value() {
        let mut hash = header_hash_internal(&genesis());
        hash.reverse();
        assert_eq!(
            hash.iter().map(|b| format!("{b:02x}")).collect::<String>(),
            GENESIS_HASH_DISPLAY
        );
    }

    #[test]
    fn genesis_meets_its_target() {
        assert!(meets_own_target(&genesis()));
    }

    #[test]
    fn tampered_header_fails_proof_of_work() {
        let mut h = genesis();
        h[76] ^= 0xff; // nonce change
        assert!(!meets_own_target(&h));
    }

    #[test]
    fn compact_target_decoding() {
        // Mainnet difficulty-1 target: 0x00000000FFFF0000...0000.
        let t = target_from_bits(0x1d00_ffff).unwrap();
        assert_eq!(&t[..6], &[0, 0, 0, 0, 0xff, 0xff]);
        assert!(t[6..].iter().all(|&b| b == 0));
        assert_eq!(target_from_bits(0x0380_0000), None); // negative
    }

    #[test]
    fn single_transaction_block_verifies_with_empty_branch() {
        // The genesis block holds one transaction, so its txid is the merkle root.
        let txid = reversed(GENESIS_TXID_DISPLAY);
        let proof = SpvProof {
            headers: vec![genesis()],
            block_index: 0,
            txid,
            merkle_branch: vec![],
            tx_index: 0,
        };
        assert_eq!(verify_spv(&proof, 1), Ok(1));
        assert_eq!(
            verify_spv(&proof, 6),
            Err(SpvError::InsufficientConfirmations { have: 1, need: 6 })
        );
    }

    #[test]
    fn wrong_txid_is_rejected() {
        let mut txid = reversed(GENESIS_TXID_DISPLAY);
        txid[0] ^= 1;
        let proof = SpvProof {
            headers: vec![genesis()],
            block_index: 0,
            txid,
            merkle_branch: vec![],
            tx_index: 0,
        };
        assert_eq!(verify_spv(&proof, 1), Err(SpvError::MerkleMismatch));
    }

    #[test]
    fn broken_linkage_is_rejected() {
        // A second header that does not reference the genesis hash. Its proof of work fails
        // first, so this checks the early rejection path.
        let proof = SpvProof {
            headers: vec![genesis(), genesis()],
            block_index: 0,
            txid: [0; 32],
            merkle_branch: vec![],
            tx_index: 0,
        };
        assert!(matches!(
            verify_spv(&proof, 1),
            Err(SpvError::BrokenLinkage(1))
        ));
    }

    #[test]
    fn empty_chain_and_bad_index_are_rejected() {
        let empty = SpvProof {
            headers: vec![],
            block_index: 0,
            txid: [0; 32],
            merkle_branch: vec![],
            tx_index: 0,
        };
        assert_eq!(verify_spv(&empty, 1), Err(SpvError::EmptyChain));
        let bad = SpvProof {
            headers: vec![genesis()],
            block_index: 3,
            ..empty
        };
        assert_eq!(verify_spv(&bad, 1), Err(SpvError::BlockIndexOutOfRange(3)));
    }
}
