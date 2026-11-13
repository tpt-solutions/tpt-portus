// SPDX-License-Identifier: MIT OR Apache-2.0
//! Plan dispatch and backup-pool escalation (DESIGN.md §5.2, §5.2.1).
//!
//! An in-process model of the relayer flow. A plan is published to the primary pool.
//! Relayers acknowledge it, and once three independent acknowledgments are in, it is
//! in-flight and can be claimed. A plan still unclaimed at the escalation block moves to the
//! backup pool. A claim above the relayer's stake cap is refused and the plan falls through to
//! the backup pool.
//!
//! Not implemented here: network transport (NATS or libp2p), the relayer registry, and the
//! timeout-breach path. Once the timeout passes with no claim, the Rollback Engine takes over
//! (§5.2 Fault Tolerance). That is outside this module.

use std::collections::HashMap;

use uuid::Uuid;

use super::diversity_check::{count_independent, Acknowledgment, MIN_INDEPENDENT_ACKS};
use super::escalation::should_escalate;
use super::relayer_pool::{may_claim, DEFAULT_COLLATERALIZATION_RATIO};

/// Which pool a plan is currently offered to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pool {
    Primary,
    Backup,
}

/// A relayer as seen by the coordinator. `bond` is in the same USD-equivalent reference unit
/// as the plan amount (§5.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relayer {
    pub id: String,
    pub operator_id: String,
    pub asn: u32,
    pub bond: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchEvent {
    Escalated { intent_id: Uuid, at_block: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimError {
    UnknownIntent(Uuid),
    AlreadyClaimed {
        by: String,
    },
    /// Not enough independent acknowledgments yet.
    NotInFlight {
        independent_acks: usize,
        required: usize,
    },
    /// The claim exceeds the relayer's stake cap. In the primary pool, the plan is
    /// escalated to the backup pool (`escalated` is then true).
    OverStakeCap {
        escalated: bool,
    },
}

struct PlanState {
    amount_ref: u128,
    accepted_block: u64,
    timeout_block: u64,
    pool: Pool,
    acks: Vec<Acknowledgment>,
    claimed_by: Option<String>,
}

pub struct Coordinator {
    min_acks: usize,
    ratio: u32,
    plans: HashMap<Uuid, PlanState>,
}

impl Default for Coordinator {
    fn default() -> Self {
        Self::new(MIN_INDEPENDENT_ACKS, DEFAULT_COLLATERALIZATION_RATIO)
    }
}

impl Coordinator {
    pub fn new(min_acks: usize, ratio: u32) -> Self {
        Self {
            min_acks,
            ratio,
            plans: HashMap::new(),
        }
    }

    /// Publish a plan to the primary pool. Publishing an already-known intent does nothing.
    pub fn publish(
        &mut self,
        intent_id: Uuid,
        amount_ref: u128,
        accepted_block: u64,
        timeout_block: u64,
    ) {
        self.plans.entry(intent_id).or_insert(PlanState {
            amount_ref,
            accepted_block,
            timeout_block,
            pool: Pool::Primary,
            acks: Vec::new(),
            claimed_by: None,
        });
    }

    /// Record an acknowledgment from `relayer`. A relayer counts once. Returns whether the plan
    /// is now in-flight.
    pub fn acknowledge(&mut self, intent_id: Uuid, relayer: &Relayer) -> Result<bool, ClaimError> {
        let min = self.min_acks;
        let plan = self
            .plans
            .get_mut(&intent_id)
            .ok_or(ClaimError::UnknownIntent(intent_id))?;
        if !plan.acks.iter().any(|a| a.relayer_id == relayer.id) {
            plan.acks.push(Acknowledgment {
                relayer_id: relayer.id.clone(),
                operator_id: relayer.operator_id.clone(),
                asn: relayer.asn,
            });
        }
        Ok(count_independent(&plan.acks) >= min)
    }

    /// Claim a plan. The plan must be in-flight, unclaimed, and within the relayer's stake cap.
    pub fn claim(
        &mut self,
        intent_id: Uuid,
        relayer: &Relayer,
        block: u64,
    ) -> Result<(), ClaimError> {
        let min = self.min_acks;
        let ratio = self.ratio;
        let plan = self
            .plans
            .get_mut(&intent_id)
            .ok_or(ClaimError::UnknownIntent(intent_id))?;

        if let Some(by) = &plan.claimed_by {
            return Err(ClaimError::AlreadyClaimed { by: by.clone() });
        }
        let independent_acks = count_independent(&plan.acks);
        if independent_acks < min {
            return Err(ClaimError::NotInFlight {
                independent_acks,
                required: min,
            });
        }
        if !may_claim(plan.amount_ref, relayer.bond, ratio) {
            let escalated = plan.pool == Pool::Primary;
            if escalated {
                plan.pool = Pool::Backup;
            }
            let _ = block; // escalation time is reported by `tick`, which owns the event
            return Err(ClaimError::OverStakeCap { escalated });
        }
        plan.claimed_by = Some(relayer.id.clone());
        Ok(())
    }

    /// Advance the clock. Unclaimed plans whose escalation block has passed move to the backup
    /// pool. Events come back sorted by intent id, so the output is deterministic.
    pub fn tick(&mut self, now_block: u64) -> Vec<DispatchEvent> {
        let mut ids: Vec<Uuid> = self.plans.keys().copied().collect();
        ids.sort();
        let mut events = Vec::new();
        for id in ids {
            let plan = self.plans.get_mut(&id).expect("id came from the map");
            if plan.pool == Pool::Primary
                && should_escalate(
                    now_block,
                    plan.accepted_block,
                    plan.timeout_block,
                    plan.claimed_by.is_some(),
                )
            {
                plan.pool = Pool::Backup;
                events.push(DispatchEvent::Escalated {
                    intent_id: id,
                    at_block: now_block,
                });
            }
        }
        events
    }

    pub fn pool_of(&self, intent_id: Uuid) -> Option<Pool> {
        self.plans.get(&intent_id).map(|p| p.pool)
    }

    pub fn claimed_by(&self, intent_id: Uuid) -> Option<&str> {
        self.plans
            .get(&intent_id)
            .and_then(|p| p.claimed_by.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relayer(id: &str, operator: &str, asn: u32, bond: u128) -> Relayer {
        Relayer {
            id: id.into(),
            operator_id: operator.into(),
            asn,
            bond,
        }
    }

    fn intent() -> Uuid {
        Uuid::from_u128(7)
    }

    /// Plan accepted at block 100 with timeout 200. Escalation is at block 150.
    fn published() -> Coordinator {
        let mut c = Coordinator::default();
        c.publish(intent(), 100, 100, 200);
        c
    }

    fn ack_three(c: &mut Coordinator) {
        for r in [
            relayer("r1", "op1", 1, 1_000),
            relayer("r2", "op2", 2, 1_000),
            relayer("r3", "op3", 3, 1_000),
        ] {
            c.acknowledge(intent(), &r).unwrap();
        }
    }

    #[test]
    fn unclaimed_plan_escalates_at_the_midpoint() {
        let mut c = published();
        assert!(c.tick(149).is_empty());
        assert_eq!(c.pool_of(intent()), Some(Pool::Primary));
        assert_eq!(
            c.tick(150),
            vec![DispatchEvent::Escalated {
                intent_id: intent(),
                at_block: 150
            }]
        );
        assert_eq!(c.pool_of(intent()), Some(Pool::Backup));
        // Escalation happens once.
        assert!(c.tick(160).is_empty());
    }

    #[test]
    fn claimed_plan_does_not_escalate() {
        let mut c = published();
        ack_three(&mut c);
        c.claim(intent(), &relayer("r1", "op1", 1, 1_000), 120)
            .unwrap();
        assert!(c.tick(180).is_empty());
        assert_eq!(c.pool_of(intent()), Some(Pool::Primary));
    }

    #[test]
    fn claim_needs_three_independent_acks() {
        let mut c = published();
        c.acknowledge(intent(), &relayer("r1", "op1", 1, 1_000))
            .unwrap();
        c.acknowledge(intent(), &relayer("r2", "op2", 2, 1_000))
            .unwrap();
        assert_eq!(
            c.claim(intent(), &relayer("r1", "op1", 1, 1_000), 120),
            Err(ClaimError::NotInFlight {
                independent_acks: 2,
                required: 3
            })
        );
    }

    #[test]
    fn sybil_acks_from_one_operator_do_not_reach_in_flight() {
        let mut c = published();
        for i in 0..5 {
            let r = relayer(&format!("s{i}"), "attacker", 100 + i, 1_000);
            c.acknowledge(intent(), &r).unwrap();
        }
        assert!(matches!(
            c.claim(intent(), &relayer("s0", "attacker", 100, 1_000), 120),
            Err(ClaimError::NotInFlight {
                independent_acks: 1,
                ..
            })
        ));
    }

    #[test]
    fn a_relayer_counts_once() {
        let mut c = published();
        let r1 = relayer("r1", "op1", 1, 1_000);
        for _ in 0..5 {
            c.acknowledge(intent(), &r1).unwrap();
        }
        assert!(matches!(
            c.claim(intent(), &r1, 120),
            Err(ClaimError::NotInFlight {
                independent_acks: 1,
                ..
            })
        ));
    }

    #[test]
    fn second_claim_is_refused() {
        let mut c = published();
        ack_three(&mut c);
        c.claim(intent(), &relayer("r1", "op1", 1, 1_000), 120)
            .unwrap();
        assert_eq!(
            c.claim(intent(), &relayer("r2", "op2", 2, 1_000), 121),
            Err(ClaimError::AlreadyClaimed { by: "r1".into() })
        );
    }

    #[test]
    fn over_cap_claim_in_primary_falls_through_to_backup() {
        let mut c = Coordinator::default();
        c.publish(intent(), 300, 100, 200);
        ack_three(&mut c);
        // Bond 50 at ratio 3 caps claims at 150, below the plan's 300.
        assert_eq!(
            c.claim(intent(), &relayer("small", "op9", 9, 50), 120),
            Err(ClaimError::OverStakeCap { escalated: true })
        );
        assert_eq!(c.pool_of(intent()), Some(Pool::Backup));
        assert_eq!(c.claimed_by(intent()), None);
    }

    #[test]
    fn over_cap_claim_in_backup_is_refused_without_escalating_again() {
        let mut c = Coordinator::default();
        c.publish(intent(), 300, 100, 200);
        ack_three(&mut c);
        c.tick(150);
        assert_eq!(
            c.claim(intent(), &relayer("small", "op9", 9, 50), 151),
            Err(ClaimError::OverStakeCap { escalated: false })
        );
    }

    #[test]
    fn unknown_intent_is_reported() {
        let mut c = Coordinator::default();
        assert_eq!(
            c.acknowledge(intent(), &relayer("r1", "op1", 1, 1)),
            Err(ClaimError::UnknownIntent(intent()))
        );
    }

    /// Failover: r2 and r3 die before acknowledging, so only r1 does. The plan cannot claim,
    /// escalates at the midpoint, and two backup relayers complete the three independent acks.
    #[test]
    fn dead_relayers_fail_over_to_backup() {
        let mut c = published();
        let r1 = relayer("r1", "op1", 1, 1_000);
        assert_eq!(c.acknowledge(intent(), &r1), Ok(false));
        assert!(matches!(
            c.claim(intent(), &r1, 120),
            Err(ClaimError::NotInFlight {
                independent_acks: 1,
                ..
            })
        ));

        let events = c.tick(150);
        assert_eq!(events.len(), 1);
        assert_eq!(c.pool_of(intent()), Some(Pool::Backup));

        assert_eq!(
            c.acknowledge(intent(), &relayer("b1", "op4", 4, 1_000)),
            Ok(false)
        );
        assert_eq!(
            c.acknowledge(intent(), &relayer("b2", "op5", 5, 1_000)),
            Ok(true)
        );
        c.claim(intent(), &relayer("b1", "op4", 4, 1_000), 155)
            .unwrap();
        assert_eq!(c.claimed_by(intent()), Some("b1"));
    }
}
