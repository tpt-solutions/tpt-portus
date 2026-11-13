// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dispatcher-level simulations (DESIGN.md §5.2, §11.4). These model relayer acknowledgments
//! as data. They do not run live relayers, so they check the decision rules only.
use tpt_portus::dispatcher::diversity_check::{
    require_diversity, Acknowledgment, MIN_INDEPENDENT_ACKS,
};
use tpt_portus::dispatcher::escalation::should_escalate;
use tpt_portus::error::PortusError;

fn ack(relayer: &str, operator: &str, asn: u32) -> Acknowledgment {
    Acknowledgment {
        relayer_id: relayer.into(),
        operator_id: operator.into(),
        asn,
    }
}

#[test]
fn sybil_relayers_behind_one_operator_cannot_fake_the_threshold() {
    // Five addresses, one operator, five different ASNs: still one independent acknowledgment.
    let acks: Vec<_> = (0..5)
        .map(|i| ack(&format!("sybil-{i}"), "attacker", 100 + i))
        .collect();
    assert_eq!(
        require_diversity(&acks, MIN_INDEPENDENT_ACKS),
        Err(PortusError::InsufficientRelayerDiversity)
    );
}

#[test]
fn sybil_relayers_sharing_one_asn_cannot_fake_the_threshold() {
    let acks = [
        ack("a", "op-1", 64500),
        ack("b", "op-2", 64500),
        ack("c", "op-3", 64500),
    ];
    assert_eq!(
        require_diversity(&acks, MIN_INDEPENDENT_ACKS),
        Err(PortusError::InsufficientRelayerDiversity)
    );
}

#[test]
fn genuine_diversity_meets_the_threshold() {
    let acks = [
        ack("a", "op-1", 64500),
        ack("b", "op-2", 64501),
        ack("c", "op-3", 64502),
    ];
    assert!(require_diversity(&acks, MIN_INDEPENDENT_ACKS).is_ok());
}

#[test]
fn killing_two_of_three_relayers_leaves_no_quorum_and_escalates() {
    // Three independent relayers acknowledge. Two then fail before claiming, so one remains.
    let all = [
        ack("r1", "op-1", 1),
        ack("r2", "op-2", 2),
        ack("r3", "op-3", 3),
    ];
    assert!(require_diversity(&all, MIN_INDEPENDENT_ACKS).is_ok());

    let survivors = &all[..1];
    assert_eq!(
        require_diversity(survivors, MIN_INDEPENDENT_ACKS),
        Err(PortusError::InsufficientRelayerDiversity)
    );
    // Accepted at block 100, timeout 200: the plan escalates to the backup pool at block 150.
    assert!(should_escalate(150, 100, 200, false));
}
