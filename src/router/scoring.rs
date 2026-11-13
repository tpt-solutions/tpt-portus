// SPDX-License-Identifier: MIT OR Apache-2.0
//! Route scoring (DESIGN.md §5.1).
//!
//! Score = `0.4 * (1 - gas_norm) + 0.4 * (1 - latency_norm) + 0.2 * success_rate`.
//!
//! Gas and latency are costs, so they are inverted before weighting. The highest score then
//! wins, which matches the spec's "select the highest-scoring route" and avoids preferring
//! expensive routes. `gas_norm` and `latency_norm` are min-max normalized across the
//! eligible candidates, so each lies in [0, 1]. When every candidate has the same cost, the
//! normalized value is 0 for all of them.

use super::liquidity::Route;

pub const GAS_WEIGHT: f64 = 0.4;
pub const LATENCY_WEIGHT: f64 = 0.4;
pub const SUCCESS_WEIGHT: f64 = 0.2;

/// Score each route. The returned vector is index-aligned with `routes`.
pub fn score_all(routes: &[&Route]) -> Vec<f64> {
    let (gas_min, gas_max) = min_max(routes.iter().map(|r| r.gas_cost));
    let (lat_min, lat_max) = min_max(routes.iter().map(|r| r.latency_ms));
    routes
        .iter()
        .map(|r| {
            let gas_norm = normalize(r.gas_cost, gas_min, gas_max);
            let lat_norm = normalize(r.latency_ms, lat_min, lat_max);
            GAS_WEIGHT * (1.0 - gas_norm)
                + LATENCY_WEIGHT * (1.0 - lat_norm)
                + SUCCESS_WEIGHT * r.success_rate
        })
        .collect()
}

fn min_max(values: impl Iterator<Item = u64>) -> (u64, u64) {
    values.fold((u64::MAX, u64::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

fn normalize(value: u64, min: u64, max: u64) -> f64 {
    if max == min {
        0.0
    } else {
        (value - min) as f64 / (max - min) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(id: &str, gas: u64, latency: u64, success: f64) -> Route {
        Route {
            pool_id: id.into(),
            available_liquidity: 1_000,
            gas_cost: gas,
            latency_ms: latency,
            success_rate: success,
        }
    }

    #[test]
    fn lower_cost_scores_higher_when_otherwise_equal() {
        let cheap = route("cheap", 10, 100, 0.9);
        let dear = route("dear", 90, 100, 0.9);
        let scores = score_all(&[&cheap, &dear]);
        assert!(scores[0] > scores[1]);
    }

    #[test]
    fn higher_success_scores_higher_when_costs_equal() {
        let good = route("good", 50, 50, 0.99);
        let bad = route("bad", 50, 50, 0.50);
        let scores = score_all(&[&good, &bad]);
        assert!(scores[0] > scores[1]);
        assert!((scores[0] - scores[1] - 0.2 * 0.49).abs() < 1e-12);
    }

    #[test]
    fn scores_stay_within_unit_interval() {
        let a = route("a", 0, 0, 1.0);
        let b = route("b", 1_000, 1_000, 0.0);
        for s in score_all(&[&a, &b]) {
            assert!((0.0..=1.0).contains(&s));
        }
        // The best possible route scores exactly 1.0.
        assert!((score_all(&[&a, &b])[0] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn single_candidate_has_zero_normalized_costs() {
        let only = route("only", 77, 33, 0.5);
        let s = score_all(&[&only]);
        assert!((s[0] - (0.4 + 0.4 + 0.2 * 0.5)).abs() < 1e-12);
    }
}
