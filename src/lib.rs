// SPDX-License-Identifier: MIT OR Apache-2.0
//! `tpt-portus`: pure executor for cross-chain intents issued by `tpt-statera`.
//!
//! See `DESIGN.md` for the full specification. This crate currently provides the
//! handshake types, error taxonomy, configuration loader, and the generated gRPC
//! bindings. Module bodies marked as stubs are not yet implemented.

pub mod address;
pub mod asset;
pub mod config;
pub mod dispatcher;
pub mod endpoint_gen;
pub mod engine;
pub mod error;
pub mod grpc;
pub mod metrics;
pub mod rollback;
pub mod router;
pub mod types;
pub mod verifier;

pub use error::PortusError;
pub use types::{ChainId, ExecutionReceipt, ExecutionStatus, IntentExecutionPlan};
