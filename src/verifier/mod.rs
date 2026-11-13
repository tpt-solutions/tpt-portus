// SPDX-License-Identifier: MIT OR Apache-2.0
//! Proof Verifier module (DESIGN.md §5.3). Per-chain verifiers and the verification queue.

pub mod bitcoin;
pub mod cosmos;
pub mod evm;
pub mod queue;
pub mod solana;
