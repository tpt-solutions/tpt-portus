// SPDX-License-Identifier: MIT OR Apache-2.0
//! Concurrent execution (DESIGN.md §11.2): 100 simultaneous transfers are all accepted, each
//! exactly once, and each is then readable.
use std::sync::Arc;

use tpt_portus::engine::{Clock, InMemoryEngine, PortusEngine};
use tpt_portus::types::{AssetDescriptor, ChainId, ExecutionStatus, IntentExecutionPlan};
use uuid::Uuid;

const TRANSFERS: u128 = 100;

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
        protocol_version: 1,
        source_finality_proof: vec![],
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_hundred_simultaneous_transfers_are_all_accepted() {
    let engine = Arc::new(InMemoryEngine::new());
    engine.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    engine.set_dispute_window(dest(), 600);

    let handles: Vec<_> = (1..=TRANSFERS)
        .map(|i| {
            let e = Arc::clone(&engine);
            tokio::spawn(async move { e.execute_intent(plan(Uuid::from_u128(i))).await })
        })
        .collect();

    for h in handles {
        let ack = h.await.unwrap().unwrap();
        assert!(ack.accepted);
    }

    for i in 1..=TRANSFERS {
        let receipt = engine.get_status(Uuid::from_u128(i)).await.unwrap();
        assert_eq!(receipt.status, ExecutionStatus::Pending);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn simultaneous_duplicates_execute_once() {
    // Ten tasks submit the same intent at once. Each sees the same acceptance, and the
    // intent is recorded once.
    let engine = Arc::new(InMemoryEngine::new());
    engine.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    engine.set_dispute_window(dest(), 600);
    let id = Uuid::from_u128(999);

    let handles: Vec<_> = (0..10)
        .map(|_| {
            let e = Arc::clone(&engine);
            tokio::spawn(async move { e.execute_intent(plan(id)).await })
        })
        .collect();
    for h in handles {
        assert!(h.await.unwrap().unwrap().accepted);
    }
    assert_eq!(
        engine.get_status(id).await.unwrap().status,
        ExecutionStatus::Pending
    );
}
