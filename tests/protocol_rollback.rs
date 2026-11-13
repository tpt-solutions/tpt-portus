// SPDX-License-Identifier: MIT OR Apache-2.0
//! Protocol-version rejection (DESIGN.md §12.3) and the rollback request outbox (§5.5.1).
use tpt_portus::engine::{Clock, InMemoryEngine, PortusEngine, PROTOCOL_VERSION};
use tpt_portus::error::PortusError;
use tpt_portus::rollback::dispute_window::IntentEvent;
use tpt_portus::types::{AssetDescriptor, ChainId, IntentExecutionPlan};
use uuid::Uuid;

const WINDOW: u64 = 60;
const OPENED: u64 = 1_000;

fn dest() -> ChainId {
    ChainId::Cosmos {
        chain_id: "osmosis-1".into(),
    }
}

fn plan(id: Uuid, protocol_version: u16) -> IntentExecutionPlan {
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
        dispute_window_secs: WINDOW,
        fee_budget: 1,
        cryptographic_proof: vec![],
        created_at: 0,
        protocol_version,
        source_finality_proof: vec![],
    }
}

fn engine() -> InMemoryEngine {
    let e = InMemoryEngine::new();
    e.set_clock(Clock {
        block: 10,
        secs: OPENED,
    });
    e.set_dispute_window(dest(), WINDOW);
    e
}

#[tokio::test]
async fn mismatched_protocol_version_is_rejected_outright() {
    let e = engine();
    let err = e
        .execute_intent(plan(Uuid::from_u128(1), PROTOCOL_VERSION + 1))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        PortusError::ProtocolVersionMismatch {
            expected: PROTOCOL_VERSION,
            got: PROTOCOL_VERSION + 1
        }
    );
}

#[tokio::test]
async fn rollback_request_is_queued_once_after_the_marker() {
    let e = engine();
    let id = Uuid::from_u128(2);
    e.execute_intent(plan(id, PROTOCOL_VERSION)).await.unwrap();
    e.apply_event(
        id,
        &IntentEvent::TimeoutBlockReached {
            now: 0,
            window_secs: WINDOW,
        },
    )
    .unwrap();

    // Window still open, and no marker yet: nothing to deliver.
    assert!(e.drain_rollback_requests().is_empty());

    e.set_clock(Clock {
        block: 11,
        secs: OPENED + WINDOW,
    });
    e.apply_event(id, &IntentEvent::DisputeWindowElapsed { now: 0 })
        .unwrap();
    // Window elapsed but the marker is not written yet: still nothing to deliver.
    assert!(e.drain_rollback_requests().is_empty());

    e.apply_event(id, &IntentEvent::RollbackMarkerWritten("0xmarker".into()))
        .unwrap();
    let requests = e.drain_rollback_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].intent_id, id);
    assert_eq!(requests[0].destination_chain, dest());
    assert_eq!(requests[0].marker_tx, "0xmarker");

    // Delivered once. Draining again returns nothing.
    assert!(e.drain_rollback_requests().is_empty());
}

#[tokio::test]
async fn successful_intent_queues_no_rollback() {
    let e = engine();
    let id = Uuid::from_u128(3);
    e.execute_intent(plan(id, PROTOCOL_VERSION)).await.unwrap();
    e.apply_event(
        id,
        &IntentEvent::TimeoutBlockReached {
            now: 0,
            window_secs: WINDOW,
        },
    )
    .unwrap();
    e.apply_event(id, &IntentEvent::ValidProofReceived(vec![1]))
        .unwrap();
    assert!(e.drain_rollback_requests().is_empty());
}
