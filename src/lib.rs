// SPDX-License-Identifier: MIT OR Apache-2.0
//! `tpt-portus`: pure executor for cross-chain intents issued by `tpt-statera`.
//!
//! See `DESIGN.md` for the full specification.

pub mod config;
pub mod error;
pub mod grpc;
pub mod types;

pub use error::PortusError;
pub use types::{ChainId, ExecutionReceipt, ExecutionStatus, IntentExecutionPlan};
