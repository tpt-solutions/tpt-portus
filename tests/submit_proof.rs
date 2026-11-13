// SPDX-License-Identifier: MIT OR Apache-2.0
//! `SubmitProof` protections (DESIGN.md §6.4): stake check first, then per-relayer rate limit.
use tpt_portus::engine::{Clock, InMemoryEngine, PortusEngine, ProofSubmission};
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
        protocol_version: 1,
        source_finality_proof: vec![],
    }
}

fn submission(id: Uuid, stake: &[u8]) -> ProofSubmission {
    ProofSubmission {
        intent_id: id,
        relayer_stake_ref: stake.to_vec(),
        chain_proof: vec![1, 2, 3],
        destination_chain: dest(),
    }
}

async fn engine_with_intent(rps: u32, stakes: Vec<Vec<u8>>) -> (InMemoryEngine, Uuid) {
    let e = InMemoryEngine::with_protection(stakes, rps);
    e.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    e.set_dispute_window(dest(), 600);
    let id = Uuid::from_u128(42);
    e.execute_intent(plan(id)).await.unwrap();
    (e, id)
}

#[tokio::test]
async fn unstaked_submitter_is_rejected_before_verification() {
    let (e, id) = engine_with_intent(10, vec![b"staked".to_vec()]).await;
    let ack = e.submit_proof(submission(id, b"nobody")).await.unwrap();
    assert!(!ack.accepted);
    assert_eq!(
        ack.rejection_reason.as_deref(),
        Some("relayer stake reference not registered")
    );
}

#[tokio::test]
async fn staked_relayer_is_rate_limited_per_second() {
    let (e, id) = engine_with_intent(2, vec![b"relayer-a".to_vec()]).await;
    let reasons: Vec<Option<String>> = {
        let mut out = Vec::new();
        for _ in 0..3 {
            out.push(
                e.submit_proof(submission(id, b"relayer-a"))
                    .await
                    .unwrap()
                    .rejection_reason,
            );
        }
        out
    };
    assert_eq!(
        reasons[0].as_deref(),
        Some("proof verification not implemented")
    );
    assert_eq!(
        reasons[1].as_deref(),
        Some("proof verification not implemented")
    );
    assert_eq!(reasons[2].as_deref(), Some("rate limited"));
}

#[tokio::test]
async fn rate_limit_is_per_relayer() {
    let (e, id) = engine_with_intent(1, vec![b"a".to_vec(), b"b".to_vec()]).await;
    e.submit_proof(submission(id, b"a")).await.unwrap();
    let limited = e.submit_proof(submission(id, b"a")).await.unwrap();
    assert_eq!(limited.rejection_reason.as_deref(), Some("rate limited"));
    let other = e.submit_proof(submission(id, b"b")).await.unwrap();
    assert_eq!(
        other.rejection_reason.as_deref(),
        Some("proof verification not implemented")
    );
}
