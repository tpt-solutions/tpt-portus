// SPDX-License-Identifier: MIT OR Apache-2.0
//! Engine metrics (DESIGN.md §12.2): acceptances, race detections, and rollbacks are counted.
use std::sync::Arc;

use tpt_portus::engine::{Clock, InMemoryEngine, PortusEngine};
use tpt_portus::metrics::Metrics;
use tpt_portus::rollback::dispute_window::IntentEvent;
use tpt_portus::types::{AssetDescriptor, ChainId, IntentExecutionPlan};
use uuid::Uuid;

fn dest() -> ChainId {
    ChainId::Cosmos {
        chain_id: "osmosis-1".into(),
    }
}

fn plan(id: Uuid) -> IntentExecutionPlan {
    IntentExecutionPlan {
        intent_id: id,
        source_chain: ChainId::Evm { eip155_id: 1 },
        destination_chain: dest(),
        asset: AssetDescriptor {
            symbol: "USDC".into(),
            source_contract: vec![1],
            destination_contract: vec![2],
            source_decimals: 6,
            destination_decimals: 6,
        },
        amount: 5,
        recipient_address: vec![3; 20],
        timeout_block_height: 100,
        dispute_window_secs: 60,
        fee_budget: 1,
        cryptographic_proof: vec![],
        created_at: 0,
        protocol_version: 1,
        source_finality_proof: vec![],
    }
}

#[tokio::test]
async fn counters_track_acceptance_races_and_rollbacks() {
    let metrics = Arc::new(Metrics::new().unwrap());
    let engine = InMemoryEngine::new().with_metrics(Arc::clone(&metrics));
    engine.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    engine.set_dispute_window(dest(), 60);

    // Intent A: accepted, times out, late proof arrives after close (a race), then rolls back.
    let a = Uuid::from_u128(1);
    engine.execute_intent(plan(a)).await.unwrap();
    engine
        .apply_event(
            a,
            &IntentEvent::TimeoutBlockReached {
                now: 0,
                window_secs: 60,
            },
        )
        .unwrap();
    engine.set_clock(Clock {
        block: 11,
        secs: 1061,
    });
    assert!(engine
        .apply_event(a, &IntentEvent::ValidProofReceived(vec![9]))
        .is_err());
    engine
        .apply_event(a, &IntentEvent::RollbackMarkerWritten("0xm".into()))
        .unwrap();

    // Intent B: accepted only.
    engine
        .execute_intent(plan(Uuid::from_u128(2)))
        .await
        .unwrap();

    assert_eq!(metrics.intents_total.get(), 2);
    assert_eq!(metrics.dispute_window_races_detected_total.get(), 1);
    assert_eq!(metrics.rollbacks_total.get(), 1);
    assert_eq!(metrics.proof_verification_failures_total.get(), 0);
    assert!(metrics
        .render()
        .contains("portus_dispute_window_races_detected_total 1"));
}

#[tokio::test]
async fn engine_without_metrics_still_works() {
    let engine = InMemoryEngine::new();
    engine.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    engine.set_dispute_window(dest(), 60);
    assert!(
        engine
            .execute_intent(plan(Uuid::from_u128(3)))
            .await
            .unwrap()
            .accepted
    );
}
