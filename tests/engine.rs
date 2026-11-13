// SPDX-License-Identifier: MIT OR Apache-2.0
use tpt_portus::engine::{Clock, InMemoryEngine, PortusEngine, ProofSubmission};
use tpt_portus::error::PortusError;
use tpt_portus::rollback::dispute_window::{IntentEvent, IntentState};
use tpt_portus::types::{AssetDescriptor, ChainId, ExecutionStatus, IntentExecutionPlan};
use uuid::Uuid;

fn plan(id: Uuid, amount: u64, timeout: u64) -> IntentExecutionPlan {
    IntentExecutionPlan {
        intent_id: id,
        source_chain: ChainId::Evm { eip155_id: 1 },
        destination_chain: ChainId::Cosmos {
            chain_id: "osmosis-1".into(),
        },
        asset: AssetDescriptor {
            symbol: "USDC".into(),
            source_contract: vec![1],
            destination_contract: vec![2],
            source_decimals: 6,
            destination_decimals: 6,
        },
        amount,
        recipient_address: vec![3; 20],
        timeout_block_height: timeout,
        dispute_window_secs: 600,
        fee_budget: 1,
        cryptographic_proof: vec![],
        created_at: 0,
        protocol_version: 1,
        source_finality_proof: vec![],
    }
}

fn engine() -> InMemoryEngine {
    let e = InMemoryEngine::new();
    e.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    e
}

#[tokio::test]
async fn accepts_valid_plan_and_reports_pending() {
    let e = engine();
    let id = Uuid::from_u128(1);
    let ack = e.execute_intent(plan(id, 5, 100)).await.unwrap();
    assert!(ack.accepted);
    let receipt = e.get_status(id).await.unwrap();
    assert_eq!(receipt.status, ExecutionStatus::Pending);
}

#[tokio::test]
async fn duplicate_identical_plan_is_idempotent() {
    let e = engine();
    let id = Uuid::from_u128(2);
    let p = plan(id, 5, 100);
    e.execute_intent(p.clone()).await.unwrap();
    let again = e.execute_intent(p).await.unwrap();
    assert!(again.accepted);
}

#[tokio::test]
async fn same_id_different_plan_is_a_replay() {
    let e = engine();
    let id = Uuid::from_u128(3);
    e.execute_intent(plan(id, 5, 100)).await.unwrap();
    let err = e.execute_intent(plan(id, 6, 100)).await.unwrap_err();
    assert_eq!(err, PortusError::ReplayAttack(id));
}

#[tokio::test]
async fn rejects_zero_amount_and_past_timeout() {
    let e = engine();
    assert_eq!(
        e.execute_intent(plan(Uuid::from_u128(4), 0, 100))
            .await
            .unwrap_err(),
        PortusError::InvalidAmount(0)
    );
    assert_eq!(
        e.execute_intent(plan(Uuid::from_u128(5), 1, 10))
            .await
            .unwrap_err(),
        PortusError::InvalidTimeout {
            current: 10,
            timeout: 10
        }
    );
}

#[tokio::test]
async fn unknown_intent_is_not_found() {
    let e = engine();
    let id = Uuid::from_u128(6);
    assert_eq!(
        e.get_status(id).await.unwrap_err(),
        PortusError::IntentNotFound(id)
    );
}

#[tokio::test]
async fn timeout_window_then_rollback_flow() {
    let e = engine();
    let id = Uuid::from_u128(7);
    e.execute_intent(plan(id, 5, 100)).await.unwrap();

    e.apply_event(
        id,
        &IntentEvent::TimeoutBlockReached {
            now: 0,
            window_secs: 60,
        },
    )
    .unwrap();
    let r = e.get_status(id).await.unwrap();
    assert_eq!(
        r.status,
        ExecutionStatus::DisputeWindowOpen {
            closes_at_secs: 1060
        }
    );

    // Window still open: elapsing is rejected and the state is kept.
    e.set_clock(Clock {
        block: 20,
        secs: 1059,
    });
    assert!(matches!(
        e.apply_event(id, &IntentEvent::DisputeWindowElapsed { now: 0 }),
        Err(PortusError::DisputeWindowOpen {
            closes_at_secs: 1060
        })
    ));

    e.set_clock(Clock {
        block: 21,
        secs: 1060,
    });
    assert_eq!(
        e.apply_event(id, &IntentEvent::DisputeWindowElapsed { now: 0 })
            .unwrap(),
        IntentState::RollbackPending { closes_at: 1060 }
    );
    e.apply_event(id, &IntentEvent::RollbackMarkerWritten("0xmarker".into()))
        .unwrap();
    assert_eq!(
        e.get_status(id).await.unwrap().status,
        ExecutionStatus::RolledBack
    );
}

#[tokio::test]
async fn proofs_are_never_accepted_without_verification() {
    let e = engine();
    let id = Uuid::from_u128(8);
    e.execute_intent(plan(id, 5, 100)).await.unwrap();
    let ack = e
        .submit_proof(ProofSubmission {
            intent_id: id,
            relayer_stake_ref: vec![],
            chain_proof: vec![1, 2, 3],
            destination_chain: ChainId::Evm { eip155_id: 1 },
        })
        .await
        .unwrap();
    assert!(!ack.accepted);
    assert_eq!(
        e.get_status(id).await.unwrap().status,
        ExecutionStatus::Pending
    );
}
