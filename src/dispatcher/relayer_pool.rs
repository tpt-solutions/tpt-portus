// SPDX-License-Identifier: MIT OR Apache-2.0
//! Stake-to-claimable-amount cap (DESIGN.md §5.2.1).
//!
//! A relayer may claim a plan only if `amount <= staked_bond * collateralization_ratio`.
//! Amounts are in a common USD-equivalent reference unit. Claims over the cap fall through
//! to the backup pool (§5.2 Fault Tolerance). They are not accepted anyway.

/// Default collateralization ratio (§5.2.1).
pub const DEFAULT_COLLATERALIZATION_RATIO: u32 = 3;

/// Whether a relayer with `staked_bond_ref` may claim a plan of `plan_amount_ref`.
pub fn may_claim(plan_amount_ref: u128, staked_bond_ref: u128, ratio: u32) -> bool {
    match staked_bond_ref.checked_mul(u128::from(ratio)) {
        Some(cap) => plan_amount_ref <= cap,
        // The product only overflows for bonds far beyond any real stake, so the cap does not bind.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_allowed_up_to_bond_times_ratio() {
        assert!(may_claim(300, 100, DEFAULT_COLLATERALIZATION_RATIO));
        assert!(!may_claim(301, 100, DEFAULT_COLLATERALIZATION_RATIO));
    }

    #[test]
    fn zero_bond_or_zero_ratio_only_allows_zero_amount() {
        assert!(may_claim(0, 0, 3));
        assert!(!may_claim(1, 0, 3));
        assert!(!may_claim(1, 100, 0));
    }

    #[test]
    fn overflowing_cap_does_not_bind() {
        assert!(may_claim(u128::MAX, u128::MAX, 3));
    }
}
