// SPDX-License-Identifier: MIT OR Apache-2.0
//! Dispatcher module (DESIGN.md §5.2).
//!
//! Implemented so far: relayer acknowledgment independence, the stake cap, and the fee split.
//! NATS transport and backup-pool escalation are not implemented yet.

pub mod diversity_check;
pub mod fees;
pub mod nats_client;
pub mod relayer_pool;
