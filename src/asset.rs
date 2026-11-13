// SPDX-License-Identifier: MIT OR Apache-2.0
//! Cross-chain decimal conversion (DESIGN.md §4.2.1).
//!
//! `destination_amount = amount * 10^(destination_decimals - source_decimals)`.
//! Scaling up is exact. Scaling down truncates, and the truncation is accepted only
//! when its size in destination units is within `dust_threshold`. Rejecting it,
//! rather than silently truncating, is what the design requires.

use thiserror::Error;

use crate::types::AssetDescriptor;

/// Default dust threshold in destination-chain units (DESIGN.md §4.2.1).
pub const DEFAULT_DUST_THRESHOLD: u64 = 1;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ConversionError {
    /// The converted amount does not fit in u64, or the scale factor overflows.
    #[error("amount conversion overflows u64")]
    Overflow,
    /// Truncation loses more than the dust threshold.
    #[error(
        "conversion would lose {remainder}/{divisor} destination units, above the dust threshold"
    )]
    DustLoss { remainder: u128, divisor: u128 },
}

impl AssetDescriptor {
    /// Convert `amount` from `source_decimals` to `destination_decimals` units.
    ///
    /// The truncation check compares the discarded remainder against
    /// `dust_threshold * divisor`. In destination units, that is a loss of at most
    /// `dust_threshold`. With the default threshold of 1, any sub-unit truncation is
    /// accepted. Callers that want strict exactness should pass 0.
    pub fn convert_amount(&self, amount: u64, dust_threshold: u64) -> Result<u64, ConversionError> {
        let exp = i32::from(self.destination_decimals) - i32::from(self.source_decimals);
        if exp >= 0 {
            let factor = 10u128
                .checked_pow(exp as u32)
                .ok_or(ConversionError::Overflow)?;
            let scaled = u128::from(amount)
                .checked_mul(factor)
                .ok_or(ConversionError::Overflow)?;
            u64::try_from(scaled).map_err(|_| ConversionError::Overflow)
        } else {
            let divisor = 10u128
                .checked_pow(exp.unsigned_abs())
                .ok_or(ConversionError::Overflow)?;
            let quotient = u128::from(amount) / divisor;
            let remainder = u128::from(amount) % divisor;
            let within_dust = match u128::from(dust_threshold).checked_mul(divisor) {
                Some(limit) => remainder <= limit,
                None => true,
            };
            if !within_dust {
                return Err(ConversionError::DustLoss { remainder, divisor });
            }
            u64::try_from(quotient).map_err(|_| ConversionError::Overflow)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(src: u8, dst: u8) -> AssetDescriptor {
        AssetDescriptor {
            symbol: "TEST".into(),
            source_contract: vec![],
            destination_contract: vec![],
            source_decimals: src,
            destination_decimals: dst,
        }
    }

    #[test]
    fn scaling_up_is_exact() {
        assert_eq!(asset(6, 18).convert_amount(1, 0), Ok(1_000_000_000_000));
    }

    #[test]
    fn exact_scaling_down_is_accepted_even_with_zero_threshold() {
        assert_eq!(
            asset(18, 6).convert_amount(2_000_000_000_000_000_000, 0),
            Ok(2_000_000)
        );
    }

    #[test]
    fn sub_unit_truncation_is_accepted_at_default_threshold() {
        assert_eq!(
            asset(18, 6).convert_amount(1_000_000_000_000_000_001, DEFAULT_DUST_THRESHOLD),
            Ok(1_000_000)
        );
    }

    #[test]
    fn truncation_above_threshold_is_rejected() {
        assert_eq!(
            asset(18, 6).convert_amount(1_000_000_000_000_000_001, 0),
            Err(ConversionError::DustLoss {
                remainder: 1,
                divisor: 1_000_000_000_000
            })
        );
    }

    #[test]
    fn overflow_is_reported() {
        assert_eq!(
            asset(0, 18).convert_amount(u64::MAX, 0),
            Err(ConversionError::Overflow)
        );
    }
}
