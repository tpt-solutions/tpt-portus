// SPDX-License-Identifier: MIT OR Apache-2.0
//! `portus_execution_duration_seconds` records acceptance-to-terminal time (DESIGN.md §12.2).
use std::sync::Arc;

use tpt_portus::engine::{Clock, InMemoryEngine, PortusEngine, PROTOCOL_VERSION};
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
        dispute_window_secs: 600,
        fee_budget: 1,
        cryptographic_proof: vec![],
        created_at: 0,
        protocol_version: PROTOCOL_VERSION,
        source_finality_proof: vec![],
    }
}

#[tokio::test]
async fn success_records_elapsed_seconds_since_acceptance() {
    let metrics = Arc::new(Metrics::new().unwrap());
    let engine = InMemoryEngine::new().with_metrics(Arc::clone(&metrics));
    engine.set_clock(Clock {
        block: 10,
        secs: 1_000,
    });
    engine.set_dispute_window(dest(), 600);

    let id = Uuid::from_u128(1);
    engine.execute_intent(plan(id)).await.unwrap();
    engine
        .apply_event(
            id,
            &IntentEvent::TimeoutBlockReached {
                now: 0,
                window_secs: 600,
            },
        )
        .unwrap();

    engine.set_clock(Clock {
        block: 11,
        secs: 1_100,
    });
    engine
        .apply_event(id, &IntentEvent::ValidProofReceived(vec![7]))
        .unwrap();

    assert_eq!(metrics.execution_duration_seconds.get_sample_count(), 1);
    assert!((metrics.execution_duration_seconds.get_sample_sum() - 100.0).abs() < 1e-9);
}

#[tokio::test]
async fn pending_intents_record_nothing() {
    let metrics = Arc::new(Metrics::new().unwrap());
    let engine = InMemoryEngine::new().with_metrics(Arc::clone(&metrics));
    engine.set_clock(Clock {
        block: 10,
        secs: 1_000,
    });
    engine.set_dispute_window(dest(), 600);
    engine
        .execute_intent(plan(Uuid::from_u128(2)))
        .await
        .unwrap();
    assert_eq!(metrics.execution_duration_seconds.get_sample_count(), 0);
}
