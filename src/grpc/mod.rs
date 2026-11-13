// SPDX-License-Identifier: MIT OR Apache-2.0
//! gRPC layer: generated bindings for `tpt.portus.v1` (built from
//! `proto/portus/v1/portus.proto`) and the request-admission rate limiter (DESIGN.md §6.4).

pub mod rate_limit;

#[allow(clippy::all, missing_docs)]
pub mod v1 {
    tonic::include_proto!("tpt.portus.v1");
}
