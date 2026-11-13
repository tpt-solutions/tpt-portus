// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dispatcher module (DESIGN.md §5.2).
//!
//! Implemented: relayer acknowledgment independence, the stake cap, the fee split, the
//! backup-pool escalation timing, and an in-process coordinator that runs the publish, ack,
//! claim, and escalation flow. Network transport (NATS) is not implemented yet.

pub mod coordinator;
pub mod diversity_check;
pub mod escalation;
pub mod fees;
pub mod nats_client;
pub mod relayer_pool;
