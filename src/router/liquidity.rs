// SPDX-License-Identifier: MIT OR Apache-2.0
//! Liquidity candidates and single-hop route selection (DESIGN.md §5.1).
//!
//! The caller supplies the candidate pools for one `(asset, source_chain, destination_chain)`
//! tuple. This module filters them by liquidity, scores the rest, and picks the best. Pool
//! discovery and two-hop routing (§5.1 step 5, Phase 3) are not implemented here.

use thiserror::Error;

use super::scoring::score_all;

/// One liquidity pool that can serve the requested route.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub pool_id: String,
    pub available_liquidity: u64,
    pub gas_cost: u64,
    pub latency_ms: u64,
    /// Historical success rate in [0.0, 1.0].
    pub success_rate: f64,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum RouterError {
    #[error("no liquidity pool is registered for this route")]
    NoRoutes,
    #[error("insufficient liquidity: need {need}, best available {available}")]
    InsufficientLiquidity { need: u64, available: u64 },
    #[error("success_rate for pool {0} must be a number within [0, 1]")]
    InvalidSuccessRate(String),
}

/// Choose the highest-scoring pool that holds at least `amount`.
///
/// Ties are broken by the lexicographically smaller `pool_id`, so the choice is deterministic.
pub fn select_route(routes: &[Route], amount: u64) -> Result<&Route, RouterError> {
    if routes.is_empty() {
        return Err(RouterError::NoRoutes);
    }
    if let Some(bad) = routes
        .iter()
        .find(|r| !(0.0..=1.0).contains(&r.success_rate))
    {
        return Err(RouterError::InvalidSuccessRate(bad.pool_id.clone()));
    }

    let eligible: Vec<&Route> = routes
        .iter()
        .filter(|r| r.available_liquidity >= amount)
        .collect();
    if eligible.is_empty() {
        let best = routes
            .iter()
            .map(|r| r.available_liquidity)
            .max()
            .unwrap_or(0);
        return Err(RouterError::InsufficientLiquidity {
            need: amount,
            available: best,
        });
    }

    let scores = score_all(&eligible);
    let mut best_idx = 0;
    for i in 1..eligible.len() {
        let better = scores[i] > scores[best_idx]
            || (scores[i] == scores[best_idx] && eligible[i].pool_id < eligible[best_idx].pool_id);
        if better {
            best_idx = i;
        }
    }
    Ok(eligible[best_idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(id: &str, liquidity: u64, gas: u64, latency: u64, success: f64) -> Route {
        Route {
            pool_id: id.into(),
            available_liquidity: liquidity,
            gas_cost: gas,
            latency_ms: latency,
            success_rate: success,
        }
    }

    #[test]
    fn picks_cheapest_reliable_pool() {
        let routes = [
            pool("expensive", 1_000, 200, 300, 0.99),
            pool("cheap", 1_000, 10, 50, 0.95),
        ];
        assert_eq!(select_route(&routes, 100).unwrap().pool_id, "cheap");
    }

    #[test]
    fn skips_pools_without_enough_liquidity() {
        let routes = [
            pool("tiny-but-cheap", 10, 1, 1, 1.0),
            pool("deep", 1_000, 100, 100, 0.9),
        ];
        assert_eq!(select_route(&routes, 500).unwrap().pool_id, "deep");
    }

    #[test]
    fn reports_best_available_when_nothing_is_deep_enough() {
        let routes = [pool("a", 100, 1, 1, 1.0), pool("b", 300, 1, 1, 1.0)];
        assert_eq!(
            select_route(&routes, 500),
            Err(RouterError::InsufficientLiquidity {
                need: 500,
                available: 300
            })
        );
    }

    #[test]
    fn empty_and_invalid_inputs_are_rejected() {
        assert_eq!(select_route(&[], 1), Err(RouterError::NoRoutes));
        let bad = [pool("nan", 10, 1, 1, f64::NAN)];
        assert_eq!(
            select_route(&bad, 1),
            Err(RouterError::InvalidSuccessRate("nan".into()))
        );
    }

    #[test]
    fn ties_break_by_pool_id() {
        let routes = [pool("zeta", 10, 5, 5, 0.9), pool("alpha", 10, 5, 5, 0.9)];
        assert_eq!(select_route(&routes, 1).unwrap().pool_id, "alpha");
    }
}
