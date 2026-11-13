// SPDX-License-Identifier: MIT OR Apache-2.0
//! Relayer fee split and gas refund (DESIGN.md §5.2.1).
//!
//! `fee_budget` is split 70% relayer, 20% liquidity pool, and 10% slash reserve. The relayer
//! and liquidity shares are rounded down. The slash reserve takes the remainder, so the three
//! parts always sum to the budget exactly.

/// Result of splitting a fee budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeeSplit {
    pub relayer: u64,
    pub liquidity: u64,
    pub slash_reserve: u64,
}

/// Split `fee_budget` by percentages. Returns `None` unless the percentages sum to 100.
pub fn split_fee(
    fee_budget: u64,
    relayer_pct: u8,
    liquidity_pct: u8,
    slash_pct: u8,
) -> Option<FeeSplit> {
    let total = u16::from(relayer_pct) + u16::from(liquidity_pct) + u16::from(slash_pct);
    if total != 100 {
        return None;
    }
    let share = |pct: u8| (u128::from(fee_budget) * u128::from(pct) / 100) as u64;
    let relayer = share(relayer_pct);
    let liquidity = share(liquidity_pct);
    Some(FeeSplit {
        relayer,
        liquidity,
        slash_reserve: fee_budget - relayer - liquidity,
    })
}

/// Gas refund owed to the fee payer: the budgeted gas cost minus the actual cost, floored at
/// zero. A relayer that executes cheaper than budgeted keeps none of the difference (§5.2.1).
pub fn gas_refund(budgeted_gas_cost: u64, actual_gas_cost: u64) -> u64 {
    budgeted_gas_cost.saturating_sub(actual_gas_cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn even_budget_splits_70_20_10() {
        assert_eq!(
            split_fee(1_000, 70, 20, 10),
            Some(FeeSplit {
                relayer: 700,
                liquidity: 200,
                slash_reserve: 100
            })
        );
    }

    #[test]
    fn rounding_never_loses_or_creates_value() {
        for budget in [0u64, 1, 7, 101, 999, 1_000_003, u64::MAX / 2] {
            let s = split_fee(budget, 70, 20, 10).unwrap();
            assert_eq!(s.relayer + s.liquidity + s.slash_reserve, budget);
        }
    }

    #[test]
    fn percentages_must_sum_to_100() {
        assert_eq!(split_fee(1_000, 70, 20, 20), None);
    }

    #[test]
    fn refund_is_saved_gas_only() {
        assert_eq!(gas_refund(500, 320), 180);
        assert_eq!(gas_refund(500, 900), 0);
    }
}
