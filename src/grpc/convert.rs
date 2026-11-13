// SPDX-License-Identifier: MIT OR Apache-2.0
//! Conversions between the protobuf messages in `grpc::v1` and the crate's domain types.
//!
//! Proto to domain conversions are fallible. Malformed input becomes `InvalidArgument`, not a
//! panic. Domain to proto conversions are infallible.
//!
//! Lossy by design: the proto `ExecutionStatus` is a bare enum (DESIGN.md §8.1), while the
//! domain type carries data (§4.3). The data is not sent over the wire. The `tx_hash`, block
//! number, and revert reason are available only through the Rust API.

// tonic::Status is the gRPC error type and is large by design, so boxing it would only add
// noise to every handler.
#![allow(clippy::result_large_err)]

use tonic::Status;
use uuid::Uuid;

use super::v1 as pb;
use crate::engine::{AcceptanceAck, ProofAck};
use crate::error::PortusError;
use crate::types::{
    AssetDescriptor, BitcoinNetwork, ChainId, ExecutionReceipt, ExecutionStatus,
    IntentExecutionPlan, SolanaCluster,
};

pub fn status_from_error(err: PortusError) -> Status {
    use PortusError as E;
    match &err {
        E::IntentNotFound(_) => Status::not_found(err.to_string()),
        E::ReplayAttack(_) => Status::already_exists(err.to_string()),
        E::InvalidAddressFormat { .. }
        | E::InvalidAmount(_)
        | E::InvalidTimeout { .. }
        | E::DisputeWindowDrift { .. }
        | E::ProtocolVersionMismatch { .. } => Status::invalid_argument(err.to_string()),
        E::UnsupportedChain(_)
        | E::NoRouteAvailable { .. }
        | E::InsufficientLiquidity { .. }
        | E::InsufficientRelayerDiversity
        | E::DisputeWindowOpen { .. }
        | E::ProofVerificationFailed { .. } => Status::failed_precondition(err.to_string()),
        _ => Status::internal(err.to_string()),
    }
}

fn parse_uuid(s: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(s)
        .map_err(|_| Status::invalid_argument(format!("intent_id is not a UUID: {s}")))
}

fn solana_from_proto(value: i32) -> Result<SolanaCluster, Status> {
    match pb::SolanaCluster::try_from(value) {
        Ok(pb::SolanaCluster::SolanaMainnet) => Ok(SolanaCluster::Mainnet),
        Ok(pb::SolanaCluster::SolanaTestnet) => Ok(SolanaCluster::Testnet),
        Ok(pb::SolanaCluster::SolanaDevnet) => Ok(SolanaCluster::Devnet),
        Err(_) => Err(Status::invalid_argument("unknown Solana cluster")),
    }
}

fn bitcoin_from_proto(value: i32) -> Result<BitcoinNetwork, Status> {
    match pb::BitcoinNetwork::try_from(value) {
        Ok(pb::BitcoinNetwork::BitcoinMainnet) => Ok(BitcoinNetwork::Mainnet),
        Ok(pb::BitcoinNetwork::BitcoinTestnet) => Ok(BitcoinNetwork::Testnet),
        Ok(pb::BitcoinNetwork::BitcoinSignet) => Ok(BitcoinNetwork::Signet),
        Err(_) => Err(Status::invalid_argument("unknown Bitcoin network")),
    }
}

pub fn chain_from_proto(chain: Option<pb::ChainId>) -> Result<ChainId, Status> {
    use pb::chain_id::Chain;
    match chain.and_then(|c| c.chain) {
        Some(Chain::Evm(e)) => Ok(ChainId::Evm {
            eip155_id: e.eip155_id,
        }),
        Some(Chain::Cosmos(c)) => Ok(ChainId::Cosmos {
            chain_id: c.chain_id,
        }),
        Some(Chain::Solana(s)) => Ok(ChainId::Solana {
            cluster: solana_from_proto(s.cluster)?,
        }),
        Some(Chain::Bitcoin(b)) => Ok(ChainId::Bitcoin {
            network: bitcoin_from_proto(b.network)?,
        }),
        None => Err(Status::invalid_argument("chain id is missing")),
    }
}

pub fn chain_to_proto(chain: &ChainId) -> pb::ChainId {
    use pb::chain_id::Chain;
    let chain = match chain {
        ChainId::Evm { eip155_id } => Chain::Evm(pb::EvmChain {
            eip155_id: *eip155_id,
        }),
        ChainId::Cosmos { chain_id } => Chain::Cosmos(pb::CosmosChain {
            chain_id: chain_id.clone(),
        }),
        ChainId::Solana { cluster } => Chain::Solana(pb::SolanaChain {
            cluster: match cluster {
                SolanaCluster::Mainnet => pb::SolanaCluster::SolanaMainnet,
                SolanaCluster::Testnet => pb::SolanaCluster::SolanaTestnet,
                SolanaCluster::Devnet => pb::SolanaCluster::SolanaDevnet,
            } as i32,
        }),
        ChainId::Bitcoin { network } => Chain::Bitcoin(pb::BitcoinChain {
            network: match network {
                BitcoinNetwork::Mainnet => pb::BitcoinNetwork::BitcoinMainnet,
                BitcoinNetwork::Testnet => pb::BitcoinNetwork::BitcoinTestnet,
                BitcoinNetwork::Signet => pb::BitcoinNetwork::BitcoinSignet,
            } as i32,
        }),
    };
    pb::ChainId { chain: Some(chain) }
}

fn asset_from_proto(asset: Option<pb::AssetDescriptor>) -> Result<AssetDescriptor, Status> {
    let a = asset.ok_or_else(|| Status::invalid_argument("asset is missing"))?;
    let decimals = |v: u32, name: &str| {
        u8::try_from(v).map_err(|_| Status::invalid_argument(format!("{name} exceeds 255")))
    };
    Ok(AssetDescriptor {
        source_decimals: decimals(a.source_decimals, "source_decimals")?,
        destination_decimals: decimals(a.destination_decimals, "destination_decimals")?,
        symbol: a.symbol,
        source_contract: a.source_contract,
        destination_contract: a.destination_contract,
    })
}

pub fn plan_from_proto(p: pb::IntentExecutionPlan) -> Result<IntentExecutionPlan, Status> {
    Ok(IntentExecutionPlan {
        intent_id: parse_uuid(&p.intent_id)?,
        source_chain: chain_from_proto(p.source_chain)?,
        destination_chain: chain_from_proto(p.destination_chain)?,
        asset: asset_from_proto(p.asset)?,
        amount: p.amount,
        recipient_address: p.recipient_address,
        timeout_block_height: p.timeout_block_height,
        dispute_window_secs: p.dispute_window_secs,
        fee_budget: p.fee_budget,
        cryptographic_proof: p.cryptographic_proof,
        created_at: p.created_at,
        protocol_version: u16::try_from(p.protocol_version)
            .map_err(|_| Status::invalid_argument("protocol_version exceeds 65535"))?,
        source_finality_proof: p.source_finality_proof,
    })
}

pub fn intent_id_from_proto(s: &str) -> Result<Uuid, Status> {
    parse_uuid(s)
}

pub fn ack_to_proto(ack: &AcceptanceAck) -> pb::AcceptanceAck {
    pb::AcceptanceAck {
        intent_id: ack.intent_id.to_string(),
        accepted: ack.accepted,
        rejection_reason: ack.rejection_reason.clone().unwrap_or_default(),
    }
}

pub fn proof_ack_to_proto(ack: &ProofAck) -> pb::ProofAck {
    pb::ProofAck {
        accepted: ack.accepted,
        rejection_reason: ack.rejection_reason.clone().unwrap_or_default(),
    }
}

fn status_to_proto(status: &ExecutionStatus) -> pb::ExecutionStatus {
    match status {
        ExecutionStatus::Pending => pb::ExecutionStatus::Pending,
        ExecutionStatus::InFlight => pb::ExecutionStatus::InFlight,
        ExecutionStatus::Success { .. } => pb::ExecutionStatus::Success,
        ExecutionStatus::Failed { .. } => pb::ExecutionStatus::Failed,
        ExecutionStatus::DisputeWindowOpen { .. } => pb::ExecutionStatus::DisputeWindowOpen,
        ExecutionStatus::Timeout { .. } => pb::ExecutionStatus::Timeout,
        ExecutionStatus::RolledBack => pb::ExecutionStatus::RolledBack,
    }
}

pub fn receipt_to_proto(r: &ExecutionReceipt) -> pb::ExecutionReceipt {
    pb::ExecutionReceipt {
        intent_id: r.intent_id.to_string(),
        destination_chain: Some(chain_to_proto(&r.destination_chain)),
        status: status_to_proto(&r.status) as i32,
        chain_proof: r.chain_proof.clone(),
        executed_at_block: r.executed_at_block,
        portus_signature: r.portus_signature.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_round_trips_through_proto() {
        let chains = [
            ChainId::Evm { eip155_id: 42161 },
            ChainId::Cosmos {
                chain_id: "osmosis-1".into(),
            },
            ChainId::Solana {
                cluster: SolanaCluster::Devnet,
            },
            ChainId::Bitcoin {
                network: BitcoinNetwork::Signet,
            },
        ];
        for c in chains {
            assert_eq!(chain_from_proto(Some(chain_to_proto(&c))).unwrap(), c);
        }
    }

    #[test]
    fn missing_or_unknown_chain_is_invalid_argument() {
        assert_eq!(
            chain_from_proto(None).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
        let bad = pb::ChainId {
            chain: Some(pb::chain_id::Chain::Solana(pb::SolanaChain { cluster: 99 })),
        };
        assert_eq!(
            chain_from_proto(Some(bad)).unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn bad_uuid_is_invalid_argument() {
        assert_eq!(
            intent_id_from_proto("not-a-uuid").unwrap_err().code(),
            tonic::Code::InvalidArgument
        );
    }

    #[test]
    fn error_mapping_uses_expected_codes() {
        use tonic::Code;
        assert_eq!(
            status_from_error(PortusError::IntentNotFound(Uuid::nil())).code(),
            Code::NotFound
        );
        assert_eq!(
            status_from_error(PortusError::ReplayAttack(Uuid::nil())).code(),
            Code::AlreadyExists
        );
        assert_eq!(
            status_from_error(PortusError::InvalidAmount(0)).code(),
            Code::InvalidArgument
        );
        assert_eq!(
            status_from_error(PortusError::HsmError("x".into())).code(),
            Code::Internal
        );
    }
}
