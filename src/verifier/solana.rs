// SPDX-License-Identifier: MIT OR Apache-2.0
//! Solana finality proof: a stake-weighted Ed25519 signature set (DESIGN.md §5.3, fast tier).
//!
//! A message is final when validators holding more than two-thirds of total stake have signed
//! it. Each validator counts once, even if it submits several signatures. Signatures from keys
//! outside the validator set, and signatures that fail to verify, contribute nothing.
//!
//! The spec does not fix the signed message layout. This module verifies whatever bytes it is
//! given, so the caller must bind them to the chain state being proven.

use std::collections::{HashMap, HashSet};

use ed25519_dalek::{Signature, VerifyingKey};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validator {
    pub pubkey: [u8; 32],
    pub stake: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedVote {
    pub pubkey: [u8; 32],
    pub signature: [u8; 64],
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SolanaVerifyError {
    #[error("validator set is empty or has no stake")]
    NoStake,
    #[error("signed stake {signed} of {total} does not exceed two-thirds")]
    InsufficientStake { signed: u128, total: u128 },
}

/// Verify that validators holding more than 2/3 of total stake signed `message`.
/// Returns the verified stake on success.
pub fn verify_supermajority(
    validators: &[Validator],
    votes: &[SignedVote],
    message: &[u8],
) -> Result<u128, SolanaVerifyError> {
    let total: u128 = validators.iter().map(|v| u128::from(v.stake)).sum();
    if total == 0 {
        return Err(SolanaVerifyError::NoStake);
    }
    let stake_of: HashMap<[u8; 32], u64> = validators.iter().map(|v| (v.pubkey, v.stake)).collect();

    let mut counted: HashSet<[u8; 32]> = HashSet::new();
    let mut signed: u128 = 0;
    for vote in votes {
        let Some(&stake) = stake_of.get(&vote.pubkey) else {
            continue;
        };
        if counted.contains(&vote.pubkey) {
            continue;
        }
        let Ok(key) = VerifyingKey::from_bytes(&vote.pubkey) else {
            continue;
        };
        let sig = Signature::from_bytes(&vote.signature);
        if key.verify_strict(message, &sig).is_ok() {
            counted.insert(vote.pubkey);
            signed += u128::from(stake);
        }
    }

    if signed * 3 > total * 2 {
        Ok(signed)
    } else {
        Err(SolanaVerifyError::InsufficientStake { signed, total })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn validators_and_keys(n: u8) -> (Vec<Validator>, Vec<SigningKey>) {
        let keys: Vec<SigningKey> = (1..=n).map(key).collect();
        let validators = keys
            .iter()
            .map(|k| Validator {
                pubkey: k.verifying_key().to_bytes(),
                stake: 100,
            })
            .collect();
        (validators, keys)
    }

    fn vote(k: &SigningKey, msg: &[u8]) -> SignedVote {
        SignedVote {
            pubkey: k.verifying_key().to_bytes(),
            signature: k.sign(msg).to_bytes(),
        }
    }

    #[test]
    fn exactly_two_thirds_is_not_enough() {
        let (vals, keys) = validators_and_keys(3);
        let msg = b"slot-42";
        let votes = [vote(&keys[0], msg), vote(&keys[1], msg)];
        assert_eq!(
            verify_supermajority(&vals, &votes, msg),
            Err(SolanaVerifyError::InsufficientStake {
                signed: 200,
                total: 300
            })
        );
    }

    #[test]
    fn more_than_two_thirds_is_enough() {
        let (vals, keys) = validators_and_keys(3);
        let msg = b"slot-42";
        let votes: Vec<_> = keys.iter().map(|k| vote(k, msg)).collect();
        assert_eq!(verify_supermajority(&vals, &votes, msg), Ok(300));
    }

    #[test]
    fn duplicate_votes_count_once() {
        let (vals, keys) = validators_and_keys(3);
        let msg = b"slot-42";
        let v = vote(&keys[0], msg);
        let votes = vec![v.clone(), v.clone(), v.clone(), vote(&keys[1], msg)];
        assert!(verify_supermajority(&vals, &votes, msg).is_err());
    }

    #[test]
    fn bad_signature_and_outsider_contribute_nothing() {
        let (vals, keys) = validators_and_keys(3);
        let msg = b"slot-42";
        let mut bad = vote(&keys[0], msg);
        bad.signature[0] ^= 1;
        let outsider = vote(&key(99), msg);
        let votes = [bad, outsider, vote(&keys[1], msg), vote(&keys[2], msg)];
        // Only keys[1] and keys[2] count: 200 of 300 is not more than two-thirds.
        assert!(verify_supermajority(&vals, &votes, msg).is_err());
    }

    #[test]
    fn signature_over_other_message_does_not_count() {
        let (vals, keys) = validators_and_keys(3);
        let votes: Vec<_> = keys.iter().map(|k| vote(k, b"other")).collect();
        assert!(verify_supermajority(&vals, &votes, b"slot-42").is_err());
    }

    #[test]
    fn empty_validator_set_is_rejected() {
        assert_eq!(
            verify_supermajority(&[], &[], b"x"),
            Err(SolanaVerifyError::NoStake)
        );
    }
}
