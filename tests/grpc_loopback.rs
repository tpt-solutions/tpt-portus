// SPDX-License-Identifier: MIT OR Apache-2.0
//! End-to-end over a real loopback socket: the generated client talks to `PortusGrpc`
//! backed by the in-memory engine.
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Channel, Server};
use tonic::Code;
use tpt_portus::engine::{Clock, InMemoryEngine};
use tpt_portus::grpc::server::PortusGrpc;
use tpt_portus::grpc::v1::portus_service_client::PortusServiceClient;
use tpt_portus::grpc::v1::portus_service_server::PortusServiceServer;
use tpt_portus::grpc::v1::{
    chain_id::Chain, AssetDescriptor, ChainId, ChainQuery, CosmosChain, EvmChain, ExecutionStatus,
    HealthRequest, IntentExecutionPlan, IntentIdQuery, ProofSubmission,
};
use uuid::Uuid;

fn evm(id: u64) -> Option<ChainId> {
    Some(ChainId {
        chain: Some(Chain::Evm(EvmChain { eip155_id: id })),
    })
}

fn cosmos(id: &str) -> Option<ChainId> {
    Some(ChainId {
        chain: Some(Chain::Cosmos(CosmosChain {
            chain_id: id.into(),
        })),
    })
}

fn plan(id: Uuid) -> IntentExecutionPlan {
    IntentExecutionPlan {
        intent_id: id.to_string(),
        source_chain: evm(1),
        destination_chain: cosmos("osmosis-1"),
        asset: Some(AssetDescriptor {
            symbol: "USDC".into(),
            source_contract: vec![1],
            destination_contract: vec![2],
            source_decimals: 6,
            destination_decimals: 6,
        }),
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

async fn start_server(engine: Arc<InMemoryEngine>) -> PortusServiceClient<Channel> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(PortusServiceServer::new(PortusGrpc::new(engine)))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    PortusServiceClient::connect(format!("http://{addr}"))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_then_query_over_the_wire() {
    let engine = Arc::new(InMemoryEngine::new());
    engine.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    engine.set_dispute_window(
        tpt_portus::types::ChainId::Cosmos {
            chain_id: "osmosis-1".into(),
        },
        600,
    );
    let mut client = start_server(engine).await;

    let id = Uuid::from_u128(77);
    let ack = client.execute_intent(plan(id)).await.unwrap().into_inner();
    assert!(ack.accepted);
    assert_eq!(ack.intent_id, id.to_string());

    let receipt = client
        .get_intent_status(IntentIdQuery {
            intent_id: id.to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(receipt.status, ExecutionStatus::Pending as i32);
}

#[tokio::test(flavor = "multi_thread")]
async fn wire_errors_carry_grpc_codes() {
    let engine = Arc::new(InMemoryEngine::new());
    engine.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    engine.set_dispute_window(
        tpt_portus::types::ChainId::Cosmos {
            chain_id: "osmosis-1".into(),
        },
        600,
    );
    let mut client = start_server(engine).await;

    // Unknown intent.
    let err = client
        .get_intent_status(IntentIdQuery {
            intent_id: Uuid::from_u128(1).to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);

    // Malformed id.
    let err = client
        .get_intent_status(IntentIdQuery {
            intent_id: "nope".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    // Drift: plan embeds 300s, config says 600s.
    let mut drifted = plan(Uuid::from_u128(2));
    drifted.dispute_window_secs = 300;
    let err = client.execute_intent(drifted).await.unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test(flavor = "multi_thread")]
async fn dispute_window_health_proof_and_streaming_endpoints() {
    let engine = Arc::new(InMemoryEngine::new());
    engine.set_clock(Clock {
        block: 10,
        secs: 1000,
    });
    engine.set_dispute_window(
        tpt_portus::types::ChainId::Cosmos {
            chain_id: "osmosis-1".into(),
        },
        600,
    );
    let mut client = start_server(engine).await;

    let window = client
        .get_dispute_window(ChainQuery {
            chain_id: cosmos("osmosis-1"),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(window.dispute_window_secs, 600);
    assert_eq!(window.config_version, 1);

    let health = client
        .health_check(HealthRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(health.healthy);
    assert_eq!(
        health.component_status.get("engine").map(String::as_str),
        Some("ok")
    );

    // Proof for an intent that was never submitted: NotFound, not an ack rejection.
    let proof = client
        .submit_proof(ProofSubmission {
            intent_id: Uuid::from_u128(3).to_string(),
            relayer_stake_ref: vec![9],
            chain_proof: vec![],
            destination_chain: cosmos("osmosis-1"),
        })
        .await
        .unwrap_err();
    assert_eq!(proof.code(), Code::NotFound); // intent does not exist yet

    let err = client
        .watch_intent_status(IntentIdQuery {
            intent_id: Uuid::from_u128(3).to_string(),
        })
        .await
        .err()
        .unwrap();
    assert_eq!(err.code(), Code::Unimplemented);
}
