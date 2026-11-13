// SPDX-License-Identifier: MIT OR Apache-2.0
//! Relayer acknowledgment independence (DESIGN.md §5.2).
//!
//! An acknowledgment counts toward the threshold only if its relayer has a distinct operator
//! identity and a distinct ASN. Acknowledgments that share an operator or an ASN are
//! linked. Links are transitive, so each connected group counts as one independent
//! acknowledgment.

use crate::error::PortusError;

/// Minimum number of independent acknowledgments before a plan is "in-flight" (§5.2).
pub const MIN_INDEPENDENT_ACKS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acknowledgment {
    pub relayer_id: String,
    pub operator_id: String,
    pub asn: u32,
}

/// Count independent acknowledgments: the number of connected groups, where two
/// acknowledgments are connected when they share an operator or an ASN.
pub fn count_independent(acks: &[Acknowledgment]) -> usize {
    let n = acks.len();
    let mut parent: Vec<usize> = (0..n).collect();

    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }

    for i in 0..n {
        for j in (i + 1)..n {
            let linked = acks[i].operator_id == acks[j].operator_id || acks[i].asn == acks[j].asn;
            if linked {
                let a = find(&mut parent, i);
                let b = find(&mut parent, j);
                if a != b {
                    parent[a] = b;
                }
            }
        }
    }

    (0..n).filter(|&i| find(&mut parent, i) == i).count()
}

/// Succeeds when at least `min` independent acknowledgments are present.
pub fn require_diversity(acks: &[Acknowledgment], min: usize) -> Result<(), PortusError> {
    if count_independent(acks) >= min {
        Ok(())
    } else {
        Err(PortusError::InsufficientRelayerDiversity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ack(relayer: &str, operator: &str, asn: u32) -> Acknowledgment {
        Acknowledgment {
            relayer_id: relayer.into(),
            operator_id: operator.into(),
            asn,
        }
    }

    #[test]
    fn three_distinct_operators_and_asns_meet_threshold() {
        let acks = [
            ack("r1", "op1", 1),
            ack("r2", "op2", 2),
            ack("r3", "op3", 3),
        ];
        assert_eq!(count_independent(&acks), 3);
        assert!(require_diversity(&acks, MIN_INDEPENDENT_ACKS).is_ok());
    }

    #[test]
    fn shared_operator_counts_once() {
        let acks = [
            ack("r1", "op1", 1),
            ack("r2", "op1", 2),
            ack("r3", "op3", 3),
        ];
        assert_eq!(count_independent(&acks), 2);
        assert_eq!(
            require_diversity(&acks, MIN_INDEPENDENT_ACKS),
            Err(PortusError::InsufficientRelayerDiversity)
        );
    }

    #[test]
    fn shared_asn_counts_once() {
        let acks = [
            ack("r1", "op1", 7),
            ack("r2", "op2", 7),
            ack("r3", "op3", 3),
        ];
        assert_eq!(count_independent(&acks), 2);
    }

    #[test]
    fn links_are_transitive() {
        // r1 and r2 share an operator, r2 and r3 share an ASN: one independent group.
        let acks = [
            ack("r1", "op1", 1),
            ack("r2", "op1", 2),
            ack("r3", "op3", 2),
        ];
        assert_eq!(count_independent(&acks), 1);
    }

    #[test]
    fn empty_input_has_no_independent_acks() {
        assert_eq!(count_independent(&[]), 0);
    }
}
