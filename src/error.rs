// SPDX-License-Identifier: MIT OR Apache-2.0
//! Error taxonomy (DESIGN.md §10.1).
//!
//! Variants beyond the spec's taxonomy:
//! - `IntentNotFound`: a query for an unknown intent.
//! - `DisputeWindowDrift` and `UnsupportedChain`: the §4.5 config-sync checks.

use thiserror::Error;
use uuid::Uuid;

use crate::types::ChainId;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PortusError {
    #[error("Invalid address format for chain {chain_id}")]
    InvalidAddressFormat { chain_id: ChainId },

    #[error("Amount must be > 0, got {0}")]
    InvalidAmount(u64),

    #[error("Timeout block {timeout} is not in the future (current: {current})")]
    InvalidTimeout { current: u64, timeout: u64 },

    #[error("No viable route found for {asset} from chain {src} to chain {dst}")]
    NoRouteAvailable {
        asset: String,
        src: ChainId,
        dst: ChainId,
    },

    #[error("Insufficient liquidity: need {need}, available {available}")]
    InsufficientLiquidity { need: u64, available: u64 },

    #[error("Relayer dispatch failed after {attempts} attempts")]
    RelayerDispatchFailed { attempts: u32 },

    #[error("Timeout: block {current} exceeded deadline {timeout}")]
    ExecutionTimeout { current: u64, timeout: u64 },

    #[error("Dispute window still open, closes at {closes_at_secs}")]
    DisputeWindowOpen { closes_at_secs: u64 },

    #[error("Chain proof verification failed for chain {chain_id}: {reason}")]
    ProofVerificationFailed { chain_id: ChainId, reason: String },

    #[error("Replay detected: intent_id {0} already executed")]
    ReplayAttack(Uuid),

    #[error("Relayer acknowledgment rejected: insufficient operator diversity")]
    InsufficientRelayerDiversity,

    #[error("Statera connection lost: {0}")]
    StateraUnavailable(String),

    #[error("Intent {0} not found")]
    IntentNotFound(Uuid),

    #[error("Dispute window drift: plan embeds {plan_secs}s but Portus is configured for {config_secs}s")]
    DisputeWindowDrift { plan_secs: u64, config_secs: u64 },

    #[error("No dispute window is configured for chain {0}")]
    UnsupportedChain(ChainId),

    #[error("protocol version {got} is not supported; this build accepts {expected}")]
    ProtocolVersionMismatch { expected: u16, got: u16 },

    #[error("HSM signing failure: {0}")]
    HsmError(String),
}
