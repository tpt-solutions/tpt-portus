// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dispute-window race (DESIGN.md §5.5.1, §11.2): a valid proof one second before the window
//! closes must succeed the intent, and one arriving one second after must not.
use tpt_portus::engine::{Clock, InMemoryEngine, PortusEngine};
use tpt_portus::error::PortusError;
use tpt_portus::rollback::dispute_window::{IntentEvent, IntentState};
use tpt_portus::types::{AssetDescriptor, ChainId, ExecutionStatus, IntentExecutionPlan};
use uuid::Uuid;

const WINDOW_SECS: u64 = 60;
const OPENED_AT: u64 = 1_000;
const CLOSES_AT: u64 = OPENED_AT + WINDOW_SECS;

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
        dispute_window_secs: WINDOW_SECS,
        fee_budget: 1,
        cryptographic_proof: vec![],
        created_at: 0,
        protocol_version: 1,
        source_finality_proof: vec![],
    }
}

/// An intent whose window opened at `OPENED_AT` and closes at `CLOSES_AT`.
async fn timed_out_intent(id: Uuid) -> InMemoryEngine {
    let e = InMemoryEngine::new();
    e.set_clock(Clock {
        block: 10,
        secs: OPENED_AT,
    });
    e.set_dispute_window(dest(), WINDOW_SECS);
    e.execute_intent(plan(id)).await.unwrap();
    e.apply_event(
        id,
        &IntentEvent::TimeoutBlockReached {
            now: 0,
            window_secs: WINDOW_SECS,
        },
    )
    .unwrap();
    e
}

#[tokio::test]
async fn proof_one_second_before_close_succeeds_the_intent() {
    let id = Uuid::from_u128(1);
    let e = timed_out_intent(id).await;
    e.set_clock(Clock {
        block: 11,
        secs: CLOSES_AT - 1,
    });
    let state = e
        .apply_event(id, &IntentEvent::ValidProofReceived(vec![7]))
        .unwrap();
    assert_eq!(state, IntentState::Success { proof: vec![7] });
    assert!(matches!(
        e.get_status(id).await.unwrap().status,
        ExecutionStatus::Success { .. }
    ));
}

#[tokio::test]
async fn proof_one_second_after_close_is_rejected_without_an_elapsed_event() {
    // No DisputeWindowElapsed is sent. The clock alone must close the window, so the late
    // proof cannot race the rollback.
    let id = Uuid::from_u128(2);
    let e = timed_out_intent(id).await;
    e.set_clock(Clock {
        block: 11,
        secs: CLOSES_AT + 1,
    });
    let err = e
        .apply_event(id, &IntentEvent::ValidProofReceived(vec![7]))
        .unwrap_err();
    assert!(matches!(err, PortusError::ProofVerificationFailed { .. }));
    assert_eq!(
        e.get_status(id).await.unwrap().status,
        ExecutionStatus::Timeout {
            current_block: 11,
            timeout_block: 100
        }
    );
}

#[tokio::test]
async fn proof_exactly_at_close_is_rejected() {
    let id = Uuid::from_u128(3);
    let e = timed_out_intent(id).await;
    e.set_clock(Clock {
        block: 11,
        secs: CLOSES_AT,
    });
    assert!(e
        .apply_event(id, &IntentEvent::ValidProofReceived(vec![7]))
        .is_err());
}

#[tokio::test]
async fn elapsed_event_after_clock_settlement_is_idempotent() {
    let id = Uuid::from_u128(4);
    let e = timed_out_intent(id).await;
    e.set_clock(Clock {
        block: 11,
        secs: CLOSES_AT,
    });
    let first = e
        .apply_event(id, &IntentEvent::DisputeWindowElapsed { now: 0 })
        .unwrap();
    let second = e
        .apply_event(id, &IntentEvent::DisputeWindowElapsed { now: 0 })
        .unwrap();
    assert_eq!(
        first,
        IntentState::RollbackPending {
            closes_at: CLOSES_AT
        }
    );
    assert_eq!(first, second);
}
