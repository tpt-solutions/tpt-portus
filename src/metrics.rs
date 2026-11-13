// SPDX-License-Identifier: MIT OR Apache-2.0
//! Prometheus metrics (DESIGN.md §12.2).
//!
//! Implemented: the counters, the gauge, and `portus_execution_duration_seconds`, which the
//! engine updates. `portus_relayer_claim_duration_seconds` needs relayer claims, which are not
//! implemented yet. Alerting rules are in `deploy/prometheus/portus-alerts.yml`.

use prometheus::{Encoder, Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder};

pub struct Metrics {
    registry: Registry,
    /// Plans accepted for execution.
    pub intents_total: IntCounter,
    /// `ROLLED_BACK` markers written.
    pub rollbacks_total: IntCounter,
    /// Valid proofs that arrived after the dispute window closed. Any increment is alert-worthy.
    pub dispute_window_races_detected_total: IntCounter,
    /// Proofs that failed verification. Not yet incremented, because verification is not implemented.
    pub proof_verification_failures_total: IntCounter,
    /// Available liquidity, set by the router's pool registry when it exists.
    pub liquidity_available: IntGauge,
    /// Seconds from acceptance to a terminal state (success, failure, or rollback).
    pub execution_duration_seconds: Histogram,
}

impl Metrics {
    pub fn new() -> prometheus::Result<Self> {
        let registry = Registry::new();
        let execution_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "portus_execution_duration_seconds",
                "Seconds from acceptance to a terminal intent state",
            )
            .buckets(vec![
                60.0, 300.0, 600.0, 900.0, 1_800.0, 3_600.0, 7_200.0, 14_400.0,
            ]),
        )?;
        let metrics = Self {
            intents_total: IntCounter::new("portus_intents_total", "Plans accepted for execution")?,
            rollbacks_total: IntCounter::new(
                "portus_rollbacks_total",
                "ROLLED_BACK markers written",
            )?,
            dispute_window_races_detected_total: IntCounter::new(
                "portus_dispute_window_races_detected_total",
                "Valid proofs that arrived after the dispute window closed",
            )?,
            proof_verification_failures_total: IntCounter::new(
                "portus_proof_verification_failures_total",
                "Proofs that failed verification",
            )?,
            liquidity_available: IntGauge::new(
                "portus_liquidity_available",
                "Available liquidity across registered pools",
            )?,
            execution_duration_seconds,
            registry,
        };
        metrics
            .registry
            .register(Box::new(metrics.intents_total.clone()))?;
        metrics
            .registry
            .register(Box::new(metrics.rollbacks_total.clone()))?;
        metrics.registry.register(Box::new(
            metrics.dispute_window_races_detected_total.clone(),
        ))?;
        metrics
            .registry
            .register(Box::new(metrics.proof_verification_failures_total.clone()))?;
        metrics
            .registry
            .register(Box::new(metrics.liquidity_available.clone()))?;
        metrics
            .registry
            .register(Box::new(metrics.execution_duration_seconds.clone()))?;
        Ok(metrics)
    }

    /// Render all metrics in the Prometheus text exposition format.
    pub fn render(&self) -> String {
        let mut buf = Vec::new();
        TextEncoder::new()
            .encode(&self.registry.gather(), &mut buf)
            .expect("text encoding into a Vec cannot fail");
        String::from_utf8(buf).expect("Prometheus text format is UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_render_with_their_spec_names() {
        let m = Metrics::new().unwrap();
        m.intents_total.inc();
        m.rollbacks_total.inc_by(2);
        let text = m.render();
        assert!(text.contains("portus_intents_total 1"));
        assert!(text.contains("portus_rollbacks_total 2"));
        assert!(text.contains("portus_dispute_window_races_detected_total 0"));
        assert!(text.contains("portus_liquidity_available 0"));
        assert!(text.contains("portus_execution_duration_seconds_count 0"));
    }
}
