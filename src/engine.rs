// SPDX-License-Identifier: MIT OR Apache-2.0
//! Orchestrator trait (DESIGN.md §8.2) and an in-memory implementation.
//!
//! [`InMemoryEngine`] enforces plan validation, idempotency, and the §4.5 dispute-window
//! config sync, and it drives the dispute-window state machine. It is not durable, and it
//! does not verify proofs. A proof submission is always rejected until the verifier exists
//! (§5.3), so no intent can succeed on unverified data.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::PortusError;
use crate::rollback::dispute_window::{IntentEvent, IntentState, TransitionError};
use crate::types::{ChainId, ExecutionReceipt, ExecutionStatus, IntentExecutionPlan};

/// Acknowledgement returned when a plan is accepted. Mirrors `AcceptanceAck` in the protobuf schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceAck {
    pub intent_id: Uuid,
    pub accepted: bool,
    pub rejection_reason: Option<String>,
}

/// Proof submission from a relayer. Mirrors `ProofSubmission` in the protobuf schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofSubmission {
    pub intent_id: Uuid,
    pub relayer_stake_ref: Vec<u8>,
    pub chain_proof: Vec<u8>,
    pub destination_chain: ChainId,
}

/// Result of a proof submission. Mirrors `ProofAck` in the protobuf schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofAck {
    pub accepted: bool,
    pub rejection_reason: Option<String>,
}

/// Portus-owned dispute window for one chain (§4.5). Mirrors `DisputeWindowConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisputeWindow {
    pub dispute_window_secs: u64,
    /// Bumped on every operator change, so Statera can detect and log config changes.
    pub config_version: u64,
}

#[async_trait]
pub trait PortusEngine: Send + Sync {
    async fn execute_intent(&self, plan: IntentExecutionPlan)
        -> Result<AcceptanceAck, PortusError>;
    async fn get_status(&self, intent_id: Uuid) -> Result<ExecutionReceipt, PortusError>;
    async fn get_dispute_window(&self, chain: &ChainId) -> Result<DisputeWindow, PortusError>;
    async fn submit_proof(&self, submission: ProofSubmission) -> Result<ProofAck, PortusError>;
    async fn shutdown(&self) -> Result<(), PortusError>;
}

/// Chain height and wall-clock seconds, supplied by the caller.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Clock {
    pub block: u64,
    pub secs: u64,
}

struct Record {
    plan: IntentExecutionPlan,
    state: IntentState,
    /// Block height at which the record reached `Success`.
    success_block: u64,
}

#[derive(Default)]
struct Inner {
    clock: Clock,
    windows: HashMap<ChainId, DisputeWindow>,
    records: HashMap<Uuid, Record>,
}

/// Non-durable engine for tests and local development.
#[derive(Default)]
pub struct InMemoryEngine {
    inner: Mutex<Inner>,
}

impl InMemoryEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_clock(&self, clock: Clock) {
        self.lock().clock = clock;
    }

    /// Set the dispute window for a destination chain. Each call bumps the config version.
    pub fn set_dispute_window(&self, chain: ChainId, dispute_window_secs: u64) {
        let mut inner = self.lock();
        let version = inner
            .windows
            .get(&chain)
            .map_or(1, |w| w.config_version + 1);
        inner.windows.insert(
            chain,
            DisputeWindow {
                dispute_window_secs,
                config_version: version,
            },
        );
    }

    /// Feed an event into an intent's state machine. This stands in for the dispatcher
    /// and verifier until they exist. Rejected events leave the state unchanged.
    ///
    /// Time-bearing events (`TimeoutBlockReached`, `DisputeWindowElapsed`) take their
    /// `now` from the engine clock, so callers cannot supply a time that skips the window.
    pub fn apply_event(
        &self,
        intent_id: Uuid,
        event: &IntentEvent,
    ) -> Result<IntentState, PortusError> {
        let mut inner = self.lock();
        let clock = inner.clock;
        let record = inner
            .records
            .get_mut(&intent_id)
            .ok_or(PortusError::IntentNotFound(intent_id))?;

        let event = match event {
            IntentEvent::TimeoutBlockReached { window_secs, .. } => {
                IntentEvent::TimeoutBlockReached {
                    now: clock.secs,
                    window_secs: *window_secs,
                }
            }
            IntentEvent::DisputeWindowElapsed { .. } => {
                IntentEvent::DisputeWindowElapsed { now: clock.secs }
            }
            other => other.clone(),
        };

        match record.state.apply(&event) {
            Ok(next) => {
                if matches!(next, IntentState::Success { .. }) {
                    record.success_block = clock.block;
                }
                record.state = next.clone();
                Ok(next)
            }
            Err(TransitionError::WindowStillOpen { closes_at, .. }) => {
                Err(PortusError::DisputeWindowOpen {
                    closes_at_secs: closes_at,
                })
            }
            Err(TransitionError::InvalidTransition { from, event }) => {
                Err(PortusError::ProofVerificationFailed {
                    chain_id: record.plan.destination_chain.clone(),
                    reason: format!("event {event} not valid in state {from}"),
                })
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means another thread panicked mid-update. The records are
        // still usable, so keep serving them.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[async_trait]
impl PortusEngine for InMemoryEngine {
    async fn execute_intent(
        &self,
        plan: IntentExecutionPlan,
    ) -> Result<AcceptanceAck, PortusError> {
        let mut inner = self.lock();
        if plan.amount == 0 {
            return Err(PortusError::InvalidAmount(0));
        }
        let current = inner.clock.block;
        if plan.timeout_block_height <= current {
            return Err(PortusError::InvalidTimeout {
                current,
                timeout: plan.timeout_block_height,
            });
        }
        // §4.5: the plan must embed exactly the window Portus enforces for its destination.
        let window = *inner
            .windows
            .get(&plan.destination_chain)
            .ok_or_else(|| PortusError::UnsupportedChain(plan.destination_chain.clone()))?;
        if plan.dispute_window_secs != window.dispute_window_secs {
            return Err(PortusError::DisputeWindowDrift {
                plan_secs: plan.dispute_window_secs,
                config_secs: window.dispute_window_secs,
            });
        }
        if let Some(existing) = inner.records.get(&plan.intent_id) {
            // Same plan: idempotent, return the existing acceptance. A different plan under
            // the same id is rejected as a replay (§4.4, §10.1).
            return if existing.plan == plan {
                Ok(AcceptanceAck {
                    intent_id: plan.intent_id,
                    accepted: true,
                    rejection_reason: None,
                })
            } else {
                Err(PortusError::ReplayAttack(plan.intent_id))
            };
        }
        let intent_id = plan.intent_id;
        inner.records.insert(
            intent_id,
            Record {
                plan,
                state: IntentState::Pending,
                success_block: 0,
            },
        );
        Ok(AcceptanceAck {
            intent_id,
            accepted: true,
            rejection_reason: None,
        })
    }

    async fn get_status(&self, intent_id: Uuid) -> Result<ExecutionReceipt, PortusError> {
        let inner = self.lock();
        let record = inner
            .records
            .get(&intent_id)
            .ok_or(PortusError::IntentNotFound(intent_id))?;
        let (status, chain_proof, executed_at_block) = match &record.state {
            IntentState::Pending => (ExecutionStatus::Pending, vec![], 0),
            IntentState::InFlight { .. } => (ExecutionStatus::InFlight, vec![], 0),
            IntentState::DisputeWindowOpen { closes_at, .. } => (
                ExecutionStatus::DisputeWindowOpen {
                    closes_at_secs: *closes_at,
                },
                vec![],
                0,
            ),
            IntentState::RollbackPending { .. } => (
                ExecutionStatus::Timeout {
                    current_block: inner.clock.block,
                    timeout_block: record.plan.timeout_block_height,
                },
                vec![],
                0,
            ),
            IntentState::Success { proof } => (
                // The proof carries no transaction hash, so `tx_hash` is empty for now.
                ExecutionStatus::Success {
                    tx_hash: String::new(),
                    block_number: record.success_block,
                },
                proof.clone(),
                record.success_block,
            ),
            IntentState::Failed { error_code } => (
                ExecutionStatus::Failed {
                    error_code: *error_code,
                    revert_reason: String::new(),
                },
                vec![],
                0,
            ),
            IntentState::RolledBack { .. } => (ExecutionStatus::RolledBack, vec![], 0),
        };
        Ok(ExecutionReceipt {
            intent_id,
            destination_chain: record.plan.destination_chain.clone(),
            status,
            chain_proof,
            executed_at_block,
            // Receipt signing needs the HSM (§6.3), which is not available yet.
            portus_signature: vec![],
        })
    }

    async fn get_dispute_window(&self, chain: &ChainId) -> Result<DisputeWindow, PortusError> {
        self.lock()
            .windows
            .get(chain)
            .copied()
            .ok_or_else(|| PortusError::UnsupportedChain(chain.clone()))
    }

    async fn submit_proof(&self, submission: ProofSubmission) -> Result<ProofAck, PortusError> {
        let inner = self.lock();
        if !inner.records.contains_key(&submission.intent_id) {
            return Err(PortusError::IntentNotFound(submission.intent_id));
        }
        // Verification (§5.3) is not implemented. Reject rather than trust the proof.
        Ok(ProofAck {
            accepted: false,
            rejection_reason: Some("proof verification not implemented".into()),
        })
    }

    async fn shutdown(&self) -> Result<(), PortusError> {
        Ok(())
    }
}
