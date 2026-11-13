// SPDX-License-Identifier: MIT OR Apache-2.0
//! Two-hop routing (DESIGN.md §5.1 step 5): `src -> mid -> dst`, used only when no direct
//! route exists.
//!
//! Each leg must have liquidity of at least `amount`. Paths are compared on total gas
//! (lower is better), then total latency (lower is better), then combined success rate
//! (higher is better), then pool ids so the choice is deterministic. The spec does not define
//! a path score, so this ordering is a choice; see the todo notes.

use crate::types::ChainId;

use super::liquidity::{Route, RouterError};

/// One directed pool connecting two chains.
#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    pub from: ChainId,
    pub to: ChainId,
    pub route: Route,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TwoHopPath {
    pub first: Hop,
    pub second: Hop,
}

impl TwoHopPath {
    fn total_gas(&self) -> u64 {
        self.first
            .route
            .gas_cost
            .saturating_add(self.second.route.gas_cost)
    }

    fn total_latency(&self) -> u64 {
        self.first
            .route
            .latency_ms
            .saturating_add(self.second.route.latency_ms)
    }

    fn combined_success(&self) -> f64 {
        self.first.route.success_rate * self.second.route.success_rate
    }

    fn ids(&self) -> (&str, &str) {
        (&self.first.route.pool_id, &self.second.route.pool_id)
    }

    /// True when `self` should be preferred over `other`.
    fn beats(&self, other: &TwoHopPath) -> bool {
        if self.total_gas() != other.total_gas() {
            return self.total_gas() < other.total_gas();
        }
        if self.total_latency() != other.total_latency() {
            return self.total_latency() < other.total_latency();
        }
        let (a, b) = (self.combined_success(), other.combined_success());
        if a != b {
            return a > b;
        }
        self.ids() < other.ids()
    }
}

/// Find the best two-hop path from `src` to `dst` through a chain other than either endpoint.
pub fn best_two_hop(
    hops: &[Hop],
    src: &ChainId,
    dst: &ChainId,
    amount: u64,
) -> Result<TwoHopPath, RouterError> {
    let funded = |h: &&Hop| h.route.available_liquidity >= amount;
    let mut best: Option<TwoHopPath> = None;

    for first in hops.iter().filter(|h| &h.from == src).filter(funded) {
        // The intermediate chain must be distinct from both endpoints.
        if &first.to == src || &first.to == dst {
            continue;
        }
        for second in hops
            .iter()
            .filter(|h| h.from == first.to && &h.to == dst)
            .filter(funded)
        {
            let candidate = TwoHopPath {
                first: first.clone(),
                second: second.clone(),
            };
            let better = match best.as_ref() {
                None => true,
                Some(b) => candidate.beats(b),
            };
            if better {
                best = Some(candidate);
            }
        }
    }

    best.ok_or(RouterError::NoRoutes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SolanaCluster;

    fn eth() -> ChainId {
        ChainId::Evm { eip155_id: 1 }
    }
    fn cosmos() -> ChainId {
        ChainId::Cosmos {
            chain_id: "osmosis-1".into(),
        }
    }
    fn sol() -> ChainId {
        ChainId::Solana {
            cluster: SolanaCluster::Mainnet,
        }
    }

    fn hop(from: &ChainId, to: &ChainId, id: &str, liquidity: u64, gas: u64) -> Hop {
        Hop {
            from: from.clone(),
            to: to.clone(),
            route: Route {
                pool_id: id.into(),
                available_liquidity: liquidity,
                gas_cost: gas,
                latency_ms: 100,
                success_rate: 0.9,
            },
        }
    }

    #[test]
    fn picks_cheapest_intermediate_chain() {
        let hops = [
            hop(&eth(), &cosmos(), "e-c", 1_000, 50),
            hop(&cosmos(), &sol(), "c-s", 1_000, 50),
            hop(&eth(), &sol(), "e-s-direct-ish", 1_000, 0),
        ];
        // The direct pool is not a two-hop candidate, so only the cosmos path is considered.
        let path = best_two_hop(&hops, &eth(), &sol(), 10).unwrap();
        assert_eq!(path.first.route.pool_id, "e-c");
        assert_eq!(path.second.route.pool_id, "c-s");
    }

    #[test]
    fn prefers_lower_total_gas_between_intermediates() {
        let mid_a = ChainId::Evm { eip155_id: 137 };
        let hops = [
            hop(&eth(), &cosmos(), "e-c", 1_000, 90),
            hop(&cosmos(), &sol(), "c-s", 1_000, 90),
            hop(&eth(), &mid_a, "e-m", 1_000, 10),
            hop(&mid_a, &sol(), "m-s", 1_000, 10),
        ];
        let path = best_two_hop(&hops, &eth(), &sol(), 10).unwrap();
        assert_eq!(path.first.to, mid_a);
    }

    #[test]
    fn every_leg_must_carry_the_amount() {
        let hops = [
            hop(&eth(), &cosmos(), "e-c", 5, 1),
            hop(&cosmos(), &sol(), "c-s", 1_000, 1),
        ];
        assert_eq!(
            best_two_hop(&hops, &eth(), &sol(), 10),
            Err(RouterError::NoRoutes)
        );
    }

    #[test]
    fn intermediate_must_differ_from_endpoints() {
        // A "loop" back through the source must not count as a route.
        let hops = [
            hop(&eth(), &eth(), "loop", 1_000, 1),
            hop(&eth(), &sol(), "e-s", 1_000, 1),
        ];
        assert_eq!(
            best_two_hop(&hops, &eth(), &sol(), 1),
            Err(RouterError::NoRoutes)
        );
    }
}
