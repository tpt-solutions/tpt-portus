// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dispute-window state machine (DESIGN.md §5.5.1, Appendix D).
//!
//! Every transition is a pure function of `(current state, event)`. Time is passed in
//! as data (`now`, in seconds), so the machine can be tested exhaustively without a clock.
//!
//! Differences from the Appendix D sketch:
//! - `apply` takes `&self`, so a rejected event leaves the caller's state intact.
//! - Times are explicit `u64` seconds. The sketch used `Instant` and a `Duration::default()`
//!   placeholder, and its `DisputeWindowElapsed` arm always returned an error.
//! - `RollbackPending` is a new state. It is entered when the window has elapsed and the
//!   `ROLLED_BACK` marker has not been written yet. The sketch had no way to reach
//!   `RolledBack` after the window elapsed. In this state, late proofs are rejected, which
//!   closes the late-execution race.

use thiserror::Error;

pub type RelayerId = String;
pub type ChainProof = Vec<u8>;
pub type TxHash = String;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentState {
    Pending,
    InFlight {
        claimed_by: RelayerId,
    },
    /// Timeout fired. No funds have moved. A valid proof can still succeed the intent.
    DisputeWindowOpen {
        opened_at: u64,
        closes_at: u64,
    },
    /// The window elapsed with no valid proof. The `ROLLED_BACK` marker is not yet written.
    RollbackPending {
        closes_at: u64,
    },
    Success {
        proof: ChainProof,
    },
    Failed {
        error_code: u32,
    },
    RolledBack {
        marker_tx: TxHash,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentEvent {
    RelayerClaimed(RelayerId),
    /// `window_secs` is the destination chain's `dispute_window_secs`.
    TimeoutBlockReached {
        now: u64,
        window_secs: u64,
    },
    ValidProofReceived(ChainProof),
    UnrecoverableFailure(u32),
    /// Informational: the proof was rejected by verification. The state does not change.
    FraudulentProofRejected,
    DisputeWindowElapsed {
        now: u64,
    },
    RollbackMarkerWritten(TxHash),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// The event does not apply to the current state.
    #[error("invalid transition: event {event} in state {from}")]
    InvalidTransition {
        from: &'static str,
        event: &'static str,
    },
    /// `DisputeWindowElapsed` arrived before the window closed.
    #[error("dispute window still open at {now}, closes at {closes_at}")]
    WindowStillOpen { now: u64, closes_at: u64 },
}

/// Discriminant of [`IntentState`], used by exhaustive tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateKind {
    Pending,
    InFlight,
    DisputeWindowOpen,
    RollbackPending,
    Success,
    Failed,
    RolledBack,
}

impl StateKind {
    pub const ALL: [StateKind; 7] = [
        StateKind::Pending,
        StateKind::InFlight,
        StateKind::DisputeWindowOpen,
        StateKind::RollbackPending,
        StateKind::Success,
        StateKind::Failed,
        StateKind::RolledBack,
    ];

    pub fn name(self) -> &'static str {
        match self {
            StateKind::Pending => "Pending",
            StateKind::InFlight => "InFlight",
            StateKind::DisputeWindowOpen => "DisputeWindowOpen",
            StateKind::RollbackPending => "RollbackPending",
            StateKind::Success => "Success",
            StateKind::Failed => "Failed",
            StateKind::RolledBack => "RolledBack",
        }
    }
}

/// Discriminant of [`IntentEvent`], used by exhaustive tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    RelayerClaimed,
    TimeoutBlockReached,
    ValidProofReceived,
    UnrecoverableFailure,
    FraudulentProofRejected,
    DisputeWindowElapsed,
    RollbackMarkerWritten,
}

impl EventKind {
    pub const ALL: [EventKind; 7] = [
        EventKind::RelayerClaimed,
        EventKind::TimeoutBlockReached,
        EventKind::ValidProofReceived,
        EventKind::UnrecoverableFailure,
        EventKind::FraudulentProofRejected,
        EventKind::DisputeWindowElapsed,
        EventKind::RollbackMarkerWritten,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EventKind::RelayerClaimed => "RelayerClaimed",
            EventKind::TimeoutBlockReached => "TimeoutBlockReached",
            EventKind::ValidProofReceived => "ValidProofReceived",
            EventKind::UnrecoverableFailure => "UnrecoverableFailure",
            EventKind::FraudulentProofRejected => "FraudulentProofRejected",
            EventKind::DisputeWindowElapsed => "DisputeWindowElapsed",
            EventKind::RollbackMarkerWritten => "RollbackMarkerWritten",
        }
    }
}

impl IntentState {
    pub fn kind(&self) -> StateKind {
        match self {
            IntentState::Pending => StateKind::Pending,
            IntentState::InFlight { .. } => StateKind::InFlight,
            IntentState::DisputeWindowOpen { .. } => StateKind::DisputeWindowOpen,
            IntentState::RollbackPending { .. } => StateKind::RollbackPending,
            IntentState::Success { .. } => StateKind::Success,
            IntentState::Failed { .. } => StateKind::Failed,
            IntentState::RolledBack { .. } => StateKind::RolledBack,
        }
    }

    /// Terminal states accept no further events.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IntentState::Success { .. }
                | IntentState::Failed { .. }
                | IntentState::RolledBack { .. }
        )
    }

    /// Compute the next state. On error, `self` is unchanged.
    pub fn apply(&self, event: &IntentEvent) -> Result<IntentState, TransitionError> {
        use IntentEvent as E;
        use IntentState as S;

        let invalid = || TransitionError::InvalidTransition {
            from: self.kind().name(),
            event: event_kind(event).name(),
        };

        match (self, event) {
            (S::Pending, E::RelayerClaimed(id)) => Ok(S::InFlight {
                claimed_by: id.clone(),
            }),

            // Timeout moves to DisputeWindowOpen, not straight to rollback (§5.5.1).
            (S::Pending | S::InFlight { .. }, E::TimeoutBlockReached { now, window_secs }) => {
                Ok(S::DisputeWindowOpen {
                    opened_at: *now,
                    closes_at: now.saturating_add(*window_secs),
                })
            }

            (S::InFlight { .. }, E::ValidProofReceived(proof)) => Ok(S::Success {
                proof: proof.clone(),
            }),
            (S::InFlight { .. }, E::UnrecoverableFailure(code)) => {
                Ok(S::Failed { error_code: *code })
            }

            // A late but valid proof inside the window still succeeds the intent.
            (S::DisputeWindowOpen { .. }, E::ValidProofReceived(proof)) => Ok(S::Success {
                proof: proof.clone(),
            }),

            (S::DisputeWindowOpen { closes_at, .. }, E::DisputeWindowElapsed { now }) => {
                if now >= closes_at {
                    Ok(S::RollbackPending {
                        closes_at: *closes_at,
                    })
                } else {
                    Err(TransitionError::WindowStillOpen {
                        now: *now,
                        closes_at: *closes_at,
                    })
                }
            }

            (S::RollbackPending { .. }, E::RollbackMarkerWritten(tx)) => Ok(S::RolledBack {
                marker_tx: tx.clone(),
            }),

            _ => Err(invalid()),
        }
    }
}

fn event_kind(event: &IntentEvent) -> EventKind {
    match event {
        IntentEvent::RelayerClaimed(_) => EventKind::RelayerClaimed,
        IntentEvent::TimeoutBlockReached { .. } => EventKind::TimeoutBlockReached,
        IntentEvent::ValidProofReceived(_) => EventKind::ValidProofReceived,
        IntentEvent::UnrecoverableFailure(_) => EventKind::UnrecoverableFailure,
        IntentEvent::FraudulentProofRejected => EventKind::FraudulentProofRejected,
        IntentEvent::DisputeWindowElapsed { .. } => EventKind::DisputeWindowElapsed,
        IntentEvent::RollbackMarkerWritten(_) => EventKind::RollbackMarkerWritten,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Representative state for each kind.
    fn sample_state(kind: StateKind) -> IntentState {
        match kind {
            StateKind::Pending => IntentState::Pending,
            StateKind::InFlight => IntentState::InFlight {
                claimed_by: "relayer-1".into(),
            },
            StateKind::DisputeWindowOpen => IntentState::DisputeWindowOpen {
                opened_at: 0,
                closes_at: 100,
            },
            StateKind::RollbackPending => IntentState::RollbackPending { closes_at: 100 },
            StateKind::Success => IntentState::Success { proof: vec![1] },
            StateKind::Failed => IntentState::Failed { error_code: 7 },
            StateKind::RolledBack => IntentState::RolledBack {
                marker_tx: "0xmarker".into(),
            },
        }
    }

    /// Representative event for each kind. The window-elapsed event uses a time
    /// at which the sample window has closed, so the pair can succeed.
    fn sample_event(kind: EventKind) -> IntentEvent {
        match kind {
            EventKind::RelayerClaimed => IntentEvent::RelayerClaimed("relayer-2".into()),
            EventKind::TimeoutBlockReached => IntentEvent::TimeoutBlockReached {
                now: 50,
                window_secs: 600,
            },
            EventKind::ValidProofReceived => IntentEvent::ValidProofReceived(vec![2]),
            EventKind::UnrecoverableFailure => IntentEvent::UnrecoverableFailure(9),
            EventKind::FraudulentProofRejected => IntentEvent::FraudulentProofRejected,
            EventKind::DisputeWindowElapsed => IntentEvent::DisputeWindowElapsed { now: 100 },
            EventKind::RollbackMarkerWritten => IntentEvent::RollbackMarkerWritten("0xtx".into()),
        }
    }

    /// The complete set of accepted (state, event) pairs. Every other pair must be rejected.
    fn allowed(s: StateKind, e: EventKind) -> bool {
        use EventKind as E;
        use StateKind as S;
        matches!(
            (s, e),
            (S::Pending, E::RelayerClaimed)
                | (S::Pending, E::TimeoutBlockReached)
                | (S::InFlight, E::TimeoutBlockReached)
                | (S::InFlight, E::ValidProofReceived)
                | (S::InFlight, E::UnrecoverableFailure)
                | (S::DisputeWindowOpen, E::ValidProofReceived)
                | (S::DisputeWindowOpen, E::DisputeWindowElapsed)
                | (S::RollbackPending, E::RollbackMarkerWritten)
        )
    }

    #[test]
    fn exhaustive_state_event_cross_product() {
        for s in StateKind::ALL {
            for e in EventKind::ALL {
                let state = sample_state(s);
                let event = sample_event(e);
                // Must not panic, and must match the explicit allow-list exactly.
                let result = state.apply(&event);
                assert_eq!(
                    result.is_ok(),
                    allowed(s, e),
                    "state {} x event {}: got {:?}",
                    s.name(),
                    e.name(),
                    result
                );
                if let Err(err) = result {
                    // A rejected transition is explicit, never a silent no-op.
                    assert!(
                        matches!(
                            err,
                            TransitionError::InvalidTransition { .. }
                                | TransitionError::WindowStillOpen { .. }
                        ),
                        "unexpected error kind {err:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn terminal_states_accept_nothing() {
        for s in StateKind::ALL {
            let state = sample_state(s);
            if state.is_terminal() {
                for e in EventKind::ALL {
                    assert!(state.apply(&sample_event(e)).is_err());
                }
            }
        }
    }

    #[test]
    fn timeout_then_valid_late_proof_succeeds() {
        let s = IntentState::Pending
            .apply(&IntentEvent::TimeoutBlockReached {
                now: 10,
                window_secs: 60,
            })
            .unwrap();
        assert_eq!(
            s,
            IntentState::DisputeWindowOpen {
                opened_at: 10,
                closes_at: 70
            }
        );
        let s = s.apply(&IntentEvent::ValidProofReceived(vec![5])).unwrap();
        assert_eq!(s, IntentState::Success { proof: vec![5] });
    }

    #[test]
    fn window_cannot_elapse_early() {
        let s = sample_state(StateKind::DisputeWindowOpen);
        assert_eq!(
            s.apply(&IntentEvent::DisputeWindowElapsed { now: 99 }),
            Err(TransitionError::WindowStillOpen {
                now: 99,
                closes_at: 100
            })
        );
    }

    #[test]
    fn late_proof_after_window_elapsed_is_rejected() {
        let s = sample_state(StateKind::DisputeWindowOpen)
            .apply(&IntentEvent::DisputeWindowElapsed { now: 100 })
            .unwrap();
        assert_eq!(s, IntentState::RollbackPending { closes_at: 100 });
        assert!(s.apply(&IntentEvent::ValidProofReceived(vec![9])).is_err());
        let s = s
            .apply(&IntentEvent::RollbackMarkerWritten("0xm".into()))
            .unwrap();
        assert_eq!(
            s,
            IntentState::RolledBack {
                marker_tx: "0xm".into()
            }
        );
    }

    #[test]
    fn rejected_event_leaves_state_unchanged() {
        let s = sample_state(StateKind::InFlight);
        assert!(s.apply(&IntentEvent::RelayerClaimed("x".into())).is_err());
        assert_eq!(s, sample_state(StateKind::InFlight));
    }
}
