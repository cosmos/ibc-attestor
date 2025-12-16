use std::net::SocketAddr;

use tokio::sync::broadcast;
use tonic::transport::Server;
use tracing::{error, info};

use super::{health::HealthService, RpcError};
use crate::rpc::health_api::health_server::HealthServer;
use crate::rpc::health_api::FILE_DESCRIPTOR_SET;

/// Start the health check gRPC server.
///
/// This server runs independently of the main attestation server and provides
/// health check endpoints for Kubernetes readiness/liveness probes.
///
/// # Errors
///
/// Returns an error if the server fails to bind to the specified address or
/// encounters an error while serving requests.
///
/// # Panics
///
/// Panics if the embedded health proto descriptor set is invalid. This should
/// never happen as the descriptor set is validated at compile time.
#[tracing::instrument(skip_all, fields(health_addr = %health_addr))]
pub async fn start(
    health_addr: SocketAddr,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), RpcError> {
    info!(
        health_addr = %health_addr,
        "starting health check server"
    );

    // Configure reflection service for service discovery
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("building health reflection service should never fail with valid embedded descriptor set");

    let health_service = HealthService::new();

    info!(health_addr = %health_addr, "health check server ready, listening for requests");

    // Serve with graceful shutdown
    let serve_result = Server::builder()
        .add_service(HealthServer::new(health_service))
        .add_service(reflection_service)
        .serve_with_shutdown(health_addr, async move {
            let _ = shutdown_rx.recv().await;
            info!("health check server received shutdown signal");
        })
        .await;

    match serve_result {
        Ok(()) => {
            info!("health check server stopped gracefully");
            Ok(())
        }
        Err(e) => {
            error!(error = ?e, "health check server failed");
            Err(e.into())
        }
    }
}
