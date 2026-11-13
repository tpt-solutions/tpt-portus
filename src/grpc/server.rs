// SPDX-License-Identifier: MIT OR Apache-2.0
//! `PortusService` implementation over any [`PortusEngine`] (DESIGN.md §8.1).
//!
//! `WatchIntentStatus` returns `Unimplemented`. Live status streaming needs the event bus,
//! which does not exist yet. Callers should poll `GetIntentStatus` until then.

use std::sync::Arc;

use tonic::{Request, Response, Status};

use super::convert::{
    ack_to_proto, chain_from_proto, chain_to_proto, intent_id_from_proto, plan_from_proto,
    proof_ack_to_proto, receipt_to_proto, status_from_error,
};
use super::v1 as pb;
use crate::engine::{PortusEngine, ProofSubmission};
use crate::types::ChainId;

pub struct PortusGrpc<E> {
    engine: Arc<E>,
}

impl<E> PortusGrpc<E> {
    pub fn new(engine: Arc<E>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl<E: PortusEngine + 'static> pb::portus_service_server::PortusService for PortusGrpc<E> {
    type WatchIntentStatusStream = tonic::codegen::BoxStream<pb::ExecutionReceipt>;

    async fn execute_intent(
        &self,
        request: Request<pb::IntentExecutionPlan>,
    ) -> Result<Response<pb::AcceptanceAck>, Status> {
        let plan = plan_from_proto(request.into_inner())?;
        let ack = self
            .engine
            .execute_intent(plan)
            .await
            .map_err(status_from_error)?;
        Ok(Response::new(ack_to_proto(&ack)))
    }

    async fn get_intent_status(
        &self,
        request: Request<pb::IntentIdQuery>,
    ) -> Result<Response<pb::ExecutionReceipt>, Status> {
        let id = intent_id_from_proto(&request.into_inner().intent_id)?;
        let receipt = self
            .engine
            .get_status(id)
            .await
            .map_err(status_from_error)?;
        Ok(Response::new(receipt_to_proto(&receipt)))
    }

    async fn watch_intent_status(
        &self,
        _request: Request<pb::IntentIdQuery>,
    ) -> Result<Response<Self::WatchIntentStatusStream>, Status> {
        Err(Status::unimplemented(
            "streaming status needs the event bus; poll GetIntentStatus",
        ))
    }

    async fn submit_proof(
        &self,
        request: Request<pb::ProofSubmission>,
    ) -> Result<Response<pb::ProofAck>, Status> {
        let s = request.into_inner();
        let submission = ProofSubmission {
            intent_id: intent_id_from_proto(&s.intent_id)?,
            relayer_stake_ref: s.relayer_stake_ref,
            chain_proof: s.chain_proof,
            destination_chain: chain_from_proto(s.destination_chain)?,
        };
        let ack = self
            .engine
            .submit_proof(submission)
            .await
            .map_err(status_from_error)?;
        Ok(Response::new(proof_ack_to_proto(&ack)))
    }

    async fn get_dispute_window(
        &self,
        request: Request<pb::ChainQuery>,
    ) -> Result<Response<pb::DisputeWindowConfig>, Status> {
        let chain: ChainId = chain_from_proto(request.into_inner().chain_id)?;
        let window = self
            .engine
            .get_dispute_window(&chain)
            .await
            .map_err(status_from_error)?;
        Ok(Response::new(pb::DisputeWindowConfig {
            chain_id: Some(chain_to_proto(&chain)),
            dispute_window_secs: window.dispute_window_secs,
            config_version: window.config_version,
        }))
    }

    async fn health_check(
        &self,
        _request: Request<pb::HealthRequest>,
    ) -> Result<Response<pb::HealthResponse>, Status> {
        Ok(Response::new(pb::HealthResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            component_status: [("engine".to_owned(), "ok".to_owned())]
                .into_iter()
                .collect(),
        }))
    }
}
