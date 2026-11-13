// SPDX-License-Identifier: MIT OR Apache-2.0
//! Per-relayer rate limiting for `SubmitProof` (DESIGN.md §6.4).
//!
//! A token bucket per relayer. Its capacity and refill rate are both `rps` tokens per second.
//! Time is passed in as milliseconds, so the limiter is deterministic under test.
//!
//! Known limit: buckets are kept for every key seen and are never evicted. Eviction belongs
//! with the gRPC server, which knows which relayers are still staked.

use std::collections::HashMap;

/// Tokens are tracked in thousandths, so fractional refill is exact in integer arithmetic.
const MILLI: u64 = 1_000;

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens_milli: u64,
    last_ms: u64,
}

#[derive(Debug)]
pub struct RateLimiter {
    rps: u64,
    buckets: HashMap<String, Bucket>,
}

impl RateLimiter {
    pub fn new(rps: u32) -> Self {
        Self {
            rps: u64::from(rps),
            buckets: HashMap::new(),
        }
    }

    /// Admit or reject one request from `key` at time `now_ms`. A zero rate rejects everything.
    pub fn try_acquire(&mut self, key: &str, now_ms: u64) -> bool {
        let capacity = self.rps.saturating_mul(MILLI);
        let bucket = self.buckets.entry(key.to_owned()).or_insert(Bucket {
            tokens_milli: capacity,
            last_ms: now_ms,
        });

        // rps tokens per second equals rps milli-tokens per millisecond.
        let elapsed = now_ms.saturating_sub(bucket.last_ms);
        let refilled = bucket
            .tokens_milli
            .saturating_add(elapsed.saturating_mul(self.rps));
        bucket.tokens_milli = refilled.min(capacity);
        bucket.last_ms = now_ms;

        if bucket.tokens_milli >= MILLI {
            bucket.tokens_milli -= MILLI;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_is_capped_at_rps() {
        let mut rl = RateLimiter::new(3);
        let admitted = (0..10).filter(|_| rl.try_acquire("r1", 0)).count();
        assert_eq!(admitted, 3);
    }

    #[test]
    fn tokens_refill_over_time() {
        let mut rl = RateLimiter::new(10);
        for _ in 0..10 {
            assert!(rl.try_acquire("r1", 0));
        }
        assert!(!rl.try_acquire("r1", 0));
        // 100 ms at 10 rps refills exactly one token.
        assert!(rl.try_acquire("r1", 100));
        assert!(!rl.try_acquire("r1", 100));
    }

    #[test]
    fn refill_never_exceeds_capacity() {
        let mut rl = RateLimiter::new(2);
        assert!(rl.try_acquire("r1", 0));
        // A long idle period refills to capacity, not beyond it.
        let admitted = (0..10).filter(|_| rl.try_acquire("r1", 1_000_000)).count();
        assert_eq!(admitted, 2);
    }

    #[test]
    fn relayers_are_limited_independently() {
        let mut rl = RateLimiter::new(1);
        assert!(rl.try_acquire("a", 0));
        assert!(!rl.try_acquire("a", 0));
        assert!(rl.try_acquire("b", 0));
    }

    #[test]
    fn zero_rate_rejects_everything() {
        let mut rl = RateLimiter::new(0);
        assert!(!rl.try_acquire("a", 0));
        assert!(!rl.try_acquire("a", 10_000));
    }
}
