// SPDX-License-Identifier: MIT OR Apache-2.0
//! Backup-pool escalation timing (DESIGN.md §5.2 Fault Tolerance).
//!
//! The spec says: "If no relayer claims the plan within `timeout_block_height / 2`, escalate to
//! a backup relayer pool." Read literally, that is an absolute block height, and for any live
//! plan it is already in the past. This module reads it as the midpoint of the claim window,
//! from the block at which the plan was accepted to its timeout. That choice is recorded in
//! the todo notes.

/// Block at which an unclaimed plan escalates to the backup pool.
pub fn escalation_block(accepted_block: u64, timeout_block: u64) -> u64 {
    accepted_block.saturating_add(timeout_block.saturating_sub(accepted_block) / 2)
}

/// Whether an unclaimed plan should be handed to the backup pool at `now_block`.
pub fn should_escalate(
    now_block: u64,
    accepted_block: u64,
    timeout_block: u64,
    claimed: bool,
) -> bool {
    !claimed && now_block >= escalation_block(accepted_block, timeout_block)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalates_at_the_midpoint_of_the_claim_window() {
        assert_eq!(escalation_block(100, 200), 150);
        assert!(!should_escalate(149, 100, 200, false));
        assert!(should_escalate(150, 100, 200, false));
    }

    #[test]
    fn claimed_plans_never_escalate() {
        assert!(!should_escalate(199, 100, 200, true));
    }

    #[test]
    fn degenerate_window_escalates_immediately() {
        assert_eq!(escalation_block(300, 200), 300);
        assert!(should_escalate(300, 300, 200, false));
    }
}
