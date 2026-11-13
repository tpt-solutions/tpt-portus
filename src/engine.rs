// SPDX-License-Identifier: MIT OR Apache-2.0
//! Orchestrator trait (DESIGN.md §8.2) and an in-memory implementation.
//!
//! [`InMemoryEngine`] enforces plan validation, idempotency, protocol-version checks (§12.3),
//! and the §4.5 dispute-window config sync. It drives the dispute-window state machine and
//! queues a [`RollbackRequest`] for Statera when the `ROLLED_BACK` marker is written (§5.5.1).
//! It is not durable, and it does not verify proofs. A proof submission is always rejected
//! until the verifier is wired in (§5.3), so no intent can succeed on unverified data.
//!
//! Time-based settlement: when the clock reaches an open window's close, the record moves to
//! `RollbackPending` on the next access, before any event is applied. A proof that arrives
//! after the close is therefore rejected even if nobody has sent `DisputeWindowElapsed` yet.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::PortusError;
use crate::grpc::rate_limit::RateLimiter;
use crate::metrics::Metrics;
use crate::rollback::dispute_window::{IntentEvent, IntentState, TransitionError, TxHash};
use crate::types::{ChainId, ExecutionReceipt, ExecutionStatus, IntentExecutionPlan};

/// Protocol version this build accepts. Plans with any other version are rejected outright
/// (DESIGN.md §12.3).
pub const PROTOCOL_VERSION: u16 = 1;

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

/// Instruction to Statera to unlock source funds after a rollback (§5.5.1, step 2).
/// Emitted only after the `ROLLED_BACK` marker is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackRequest {
    pub intent_id: Uuid,
    pub destination_chain: ChainId,
    pub marker_tx: TxHash,
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
    /// Engine clock seconds at acceptance, for the execution-duration histogram.
    accepted_secs: u64,
}

struct Inner {
    clock: Clock,
    windows: HashMap<ChainId, DisputeWindow>,
    records: HashMap<Uuid, Record>,
    /// Stake references of registered relayers (§6.4).
    staked: HashSet<Vec<u8>>,
    limiter: RateLimiter,
    /// Rollback requests waiting to be delivered to Statera.
    outbox: Vec<RollbackRequest>,
}

/// Move an open window to `RollbackPending` once its close time has passed.
fn settle(record: &mut Record, now_secs: u64) {
    if let IntentState::DisputeWindowOpen { closes_at, .. } = record.state {
        if now_secs >= closes_at {
            record.state = IntentState::RollbackPending { closes_at };
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Non-durable engine for tests and local development.
pub struct InMemoryEngine {
    inner: Mutex<Inner>,
    metrics: Option<Arc<Metrics>>,
}

impl Default for InMemoryEngine {
    fn default() -> Self {
        Self::with_protection(std::iter::empty(), 10)
    }
}

impl InMemoryEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach Prometheus metrics. Without this the engine records nothing.
    pub fn with_metrics(mut self, metrics: Arc<Metrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Engine with a set of staked relayer references and a per-relayer `SubmitProof` rate.
    pub fn with_protection(staked: impl IntoIterator<Item = Vec<u8>>, rps: u32) -> Self {
        Self {
            metrics: None,
            inner: Mutex::new(Inner {
                clock: Clock::default(),
                windows: HashMap::new(),
                records: HashMap::new(),
                staked: staked.into_iter().collect(),
                limiter: RateLimiter::new(rps),
                outbox: Vec::new(),
            }),
        }
    }

    pub fn register_stake(&self, stake_ref: Vec<u8>) {
        self.lock().staked.insert(stake_ref);
    }

    pub fn set_clock(&self, clock: Clock) {
        self.lock().clock = clock;
    }

    /// Take every queued rollback request. Each request is returned once.
    pub fn drain_rollback_requests(&self) -> Vec<RollbackRequest> {
        std::mem::take(&mut self.lock().outbox)
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
    /// Time-bearing events take their `now` from the engine clock, so callers cannot supply
    /// a time that skips the window.
    #[tracing::instrument(skip(self, event), fields(intent_id = %intent_id))]
    pub fn apply_event(
        &self,
        intent_id: Uuid,
        event: &IntentEvent,
    ) -> Result<IntentState, PortusError> {
        let mut inner = self.lock();
        let clock = inner.clock;
        let mut emit: Option<RollbackRequest> = None;

        let result = {
            let record = inner
                .records
                .get_mut(&intent_id)
                .ok_or(PortusError::IntentNotFound(intent_id))?;
            settle(record, clock.secs);

            let event = match event {
                IntentEvent::TimeoutBlockReached { window_secs, .. } => {
                    IntentEvent::TimeoutBlockReached {
                        now: clock.secs,
                        window_secs: *window_secs,
                    }
                }
                IntentEvent::DisputeWindowElapsed { .. } => {
                    if matches!(record.state, IntentState::RollbackPending { .. }) {
                        // Already settled by the clock. Repeating the event is harmless.
                        return Ok(record.state.clone());
                    }
                    IntentEvent::DisputeWindowElapsed { now: clock.secs }
                }
                other => other.clone(),
            };

            if matches!(event, IntentEvent::ValidProofReceived(_))
                && matches!(record.state, IntentState::RollbackPending { .. })
            {
                if let Some(m) = &self.metrics {
                    m.dispute_window_races_detected_total.inc();
                }
            }

            match record.state.apply(&event) {
                Ok(next) => {
                    if let IntentState::RolledBack { marker_tx } = &next {
                        if let Some(m) = &self.metrics {
                            m.rollbacks_total.inc();
                        }
                        emit = Some(RollbackRequest {
                            intent_id,
                            destination_chain: record.plan.destination_chain.clone(),
                            marker_tx: marker_tx.clone(),
                        });
                    }
                    if matches!(next, IntentState::Success { .. }) {
                        record.success_block = clock.block;
                    }
                    if matches!(
                        next,
                        IntentState::Success { .. }
                            | IntentState::Failed { .. }
                            | IntentState::RolledBack { .. }
                    ) {
                        if let Some(m) = &self.metrics {
                            m.execution_duration_seconds
                                .observe(clock.secs.saturating_sub(record.accepted_secs) as f64);
                        }
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
        };

        // Queue the request only after the marker transition has succeeded (§5.5.1).
        if let Some(request) = emit {
            inner.outbox.push(request);
        }
        result
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means another thread panicked mid-update. The records are
        // still usable, so keep serving them.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[tracing::instrument(skip(self, plan), fields(intent_id = %plan.intent_id))]
    fn accept(&self, plan: IntentExecutionPlan) -> Result<AcceptanceAck, PortusError> {
        let mut inner = self.lock();
        if plan.protocol_version != PROTOCOL_VERSION {
            return Err(PortusError::ProtocolVersionMismatch {
                expected: PROTOCOL_VERSION,
                got: plan.protocol_version,
            });
        }
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
        if let Some(m) = &self.metrics {
            m.intents_total.inc();
        }
        let now_secs = inner.clock.secs;
        let intent_id = plan.intent_id;
        inner.records.insert(
            intent_id,
            Record {
                plan,
                state: IntentState::Pending,
                success_block: 0,
                accepted_secs: now_secs,
            },
        );
        Ok(AcceptanceAck {
            intent_id,
            accepted: true,
            rejection_reason: None,
        })
    }
}

#[async_trait]
impl PortusEngine for InMemoryEngine {
    async fn execute_intent(
        &self,
        plan: IntentExecutionPlan,
    ) -> Result<AcceptanceAck, PortusError> {
        self.accept(plan)
    }

    async fn get_status(&self, intent_id: Uuid) -> Result<ExecutionReceipt, PortusError> {
        let mut inner = self.lock();
        let now_secs = inner.clock.secs;
        let block = inner.clock.block;
        let record = inner
            .records
            .get_mut(&intent_id)
            .ok_or(PortusError::IntentNotFound(intent_id))?;
        settle(record, now_secs);

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
                    current_block: block,
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
        let mut inner = self.lock();
        if !inner.records.contains_key(&submission.intent_id) {
            return Err(PortusError::IntentNotFound(submission.intent_id));
        }
        // §6.4: the cheapest check runs first. Unstaked submitters never reach verification.
        if !inner.staked.contains(&submission.relayer_stake_ref) {
            return Ok(ProofAck {
                accepted: false,
                rejection_reason: Some("relayer stake reference not registered".into()),
            });
        }
        let now_ms = inner.clock.secs.saturating_mul(1_000);
        let key = hex(&submission.relayer_stake_ref);
        if !inner.limiter.try_acquire(&key, now_ms) {
            return Ok(ProofAck {
                accepted: false,
                rejection_reason: Some("rate limited".into()),
            });
        }
        // Verification (§5.3) is not wired in yet. Reject rather than trust the proof.
        Ok(ProofAck {
            accepted: false,
            rejection_reason: Some("proof verification not implemented".into()),
        })
    }

    async fn shutdown(&self) -> Result<(), PortusError> {
        Ok(())
    }
}
