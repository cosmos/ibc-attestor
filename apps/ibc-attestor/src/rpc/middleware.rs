use std::time::Instant;
use tonic::{Request, Response, Status};
use tracing::info;

use super::api::attestation_service_server::AttestationService;
use super::api::{
    LatestHeightRequest, LatestHeightResponse, PacketAttestationRequest, PacketAttestationResponse,
    StateAttestationRequest, StateAttestationResponse,
};
use super::attestor::AttestorService;
use crate::adapter::AttestationAdapter;
use crate::signer::Signer;

/// Logging middleware that wraps the AttestorService structured spans and request duration.
///
/// This middleware logs each RPC method call with:
/// - Adapter identification (derived from the service)
/// - Method name
/// - Request parameters (height, numPackets, commitmentType)
/// - Performance metrics (durationMs)
/// - Status (ok/error) and error messages
///
/// Errors are logged inline by the [AttestorService] and [AttestationAdapter] implementaitons
/// to simplify line tracing.
///
/// Fields in logs use *camelCase* convention for consistency.
///
/// All logs will be correlated via the trace_id provided by the OpenTelemetry layer.
pub struct LoggingMiddleware<A, S> {
    inner: AttestorService<A, S>,
}

impl<A, S> LoggingMiddleware<A, S> {
    /// Create a new LoggingAttestorService that wraps an AttestorService implementation.
    ///
    /// # Arguments
    ///
    /// * `inner` - The AttestorService implementation to wrap
    pub fn new(inner: AttestorService<A, S>) -> Self {
        Self { inner }
    }
}

#[tonic::async_trait]
impl<A, S> AttestationService for LoggingMiddleware<A, S>
where
    A: AttestationAdapter,
    S: Signer,
{
    #[tracing::instrument(skip(self, request), fields(adapter = self.inner.adapter_name(), signer = self.inner.signer_name()))]
    async fn latest_height(
        &self,
        request: Request<LatestHeightRequest>,
    ) -> Result<Response<LatestHeightResponse>, Status> {
        let start = Instant::now();
        let result = self.inner.latest_height(request).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match &result {
            Ok(response) => {
                info!(
                    height = response.get_ref().height,
                    durationMs = duration_ms,
                    status = "ok",
                );
            }
            Err(e) => {
                info!(
                    durationMs = duration_ms,
                    status = "error",
                    error = %e,
                );
            }
        }

        result
    }

    #[tracing::instrument(skip(self, request), fields(adapter = self.inner.adapter_name(), signer = self.inner.signer_name(), height = request.get_ref().height))]
    async fn state_attestation(
        &self,
        request: Request<StateAttestationRequest>,
    ) -> Result<Response<StateAttestationResponse>, Status> {
        let start = Instant::now();
        let result = self.inner.state_attestation(request).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match &result {
            Ok(response) => {
                let timestamp = response
                    .get_ref()
                    .attestation
                    .as_ref()
                    .and_then(|a| a.timestamp);
                info!(
                    timestamp = timestamp,
                    durationMs = duration_ms,
                    status = "ok",
                );
            }
            Err(e) => {
                info!(
                    durationMs = duration_ms,
                    status = "error",
                    error = %e,
                );
            }
        }

        result
    }

    #[tracing::instrument(
        skip(self, request),
        fields(
            adapter = self.inner.adapter_name(),
            signer = self.inner.signer_name(),
            height = request.get_ref().height,
            numPackets = request.get_ref().packets.len(),
            commitmentType = ?request.get_ref().commitment_type(),
        )
    )]
    async fn packet_attestation(
        &self,
        request: Request<PacketAttestationRequest>,
    ) -> Result<Response<PacketAttestationResponse>, Status> {
        let start = Instant::now();
        let result = self.inner.packet_attestation(request).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match &result {
            Ok(_) => {
                info!(durationMs = duration_ms, status = "ok",);
            }
            Err(e) => {
                info!(
                    durationMs = duration_ms,
                    status = "error",
                    error = %e,
                );
            }
        }

        result
    }
}
