// SPDX-License-Identifier: MIT OR Apache-2.0
use tpt_portus::error::PortusError;
use tpt_portus::types::{
    AssetDescriptor, BitcoinNetwork, ChainId, ExecutionReceipt, ExecutionStatus,
    IntentExecutionPlan, SolanaCluster,
};
use uuid::Uuid;

fn sample_plan() -> IntentExecutionPlan {
    IntentExecutionPlan {
        intent_id: Uuid::nil(),
        source_chain: ChainId::Evm { eip155_id: 1 },
        destination_chain: ChainId::Cosmos {
            chain_id: "osmosis-1".into(),
        },
        asset: AssetDescriptor {
            symbol: "USDC".into(),
            source_contract: vec![1; 20],
            destination_contract: vec![2; 32],
            source_decimals: 6,
            destination_decimals: 6,
        },
        amount: 1_000_000,
        recipient_address: vec![3; 20],
        timeout_block_height: 100,
        dispute_window_secs: 600,
        fee_budget: 500,
        cryptographic_proof: vec![],
        created_at: 1,
        protocol_version: 1,
        source_finality_proof: vec![],
    }
}

#[test]
fn chain_ids_distinguish_cosmos_zones_and_bitcoin() {
    assert_ne!(
        ChainId::Cosmos {
            chain_id: "cosmoshub-4".into()
        },
        ChainId::Cosmos {
            chain_id: "osmosis-1".into()
        }
    );
    assert_eq!(ChainId::Evm { eip155_id: 42161 }.to_string(), "evm:42161");
    assert_eq!(
        ChainId::Bitcoin {
            network: BitcoinNetwork::Signet
        }
        .to_string(),
        "bitcoin:Signet"
    );
    assert_eq!(
        ChainId::Solana {
            cluster: SolanaCluster::Devnet
        }
        .to_string(),
        "solana:Devnet"
    );
}

#[test]
fn plan_round_trips_through_serde() {
    let plan = sample_plan();
    let json = serde_json::to_string(&plan).unwrap();
    let back: IntentExecutionPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(plan, back);
}

#[test]
fn receipt_with_dispute_window_round_trips() {
    let receipt = ExecutionReceipt {
        intent_id: Uuid::nil(),
        destination_chain: ChainId::Evm { eip155_id: 1 },
        status: ExecutionStatus::DisputeWindowOpen { closes_at_secs: 42 },
        chain_proof: vec![],
        executed_at_block: 7,
        portus_signature: vec![],
    };
    let json = serde_json::to_string(&receipt).unwrap();
    let back: ExecutionReceipt = serde_json::from_str(&json).unwrap();
    assert_eq!(receipt, back);
}

#[test]
fn error_messages_render_chain_ids() {
    let err = PortusError::NoRouteAvailable {
        asset: "USDC".into(),
        src: ChainId::Evm { eip155_id: 1 },
        dst: ChainId::Solana {
            cluster: SolanaCluster::Mainnet,
        },
    };
    assert_eq!(
        err.to_string(),
        "No viable route found for USDC from chain evm:1 to chain solana:Mainnet"
    );
    assert_eq!(
        PortusError::InvalidAmount(0).to_string(),
        "Amount must be > 0, got 0"
    );
}
