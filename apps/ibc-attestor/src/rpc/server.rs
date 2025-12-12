use std::net::SocketAddr;

use tokio::sync::broadcast;
use tonic::transport::Server;
use tracing::{error, info};

use super::{attestor::AttestorService, tracing_interceptor, LoggingMiddleware, RpcError};
use crate::adapter::AttestationAdapter;
use crate::rpc::api::attestation_service_server::AttestationServiceServer;
use crate::rpc::api::FILE_DESCRIPTOR_SET;
use crate::signer::Signer;

/// Start the gRPC server with attestation and reflection services.
#[tracing::instrument(skip_all, fields(listen_addr = %listen_addr, adapter = adapter_name))]
pub async fn start<A, S>(
    listen_addr: SocketAddr,
    adapter: A,
    adapter_name: &'static str,
    signer: S,
    signer_name: &'static str,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(), RpcError>
where
    A: AttestationAdapter,
    S: Signer,
{
    info!(
        listenAddr = %listen_addr,
        adapter = adapter_name,
        "starting RPC server"
    );

    // Configure reflection service for service discovery
    //
    // Note: This expect is safe because the file descriptor set is embedded at compile time
    // and the build should only succeed if it's valid. This operation is infallible at runtime.
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("building reflection service should never fail with valid embedded descriptor set");

    let attestation_service = AttestorService::new(adapter, adapter_name, signer, signer_name);
    let logging_service = LoggingMiddleware::new(attestation_service);

    info!(listen_addr = %listen_addr, "gRPC server ready, listening for requests");

    // Serve with graceful shutdown
    let serve_result = Server::builder()
        .add_service(AttestationServiceServer::with_interceptor(
            logging_service,
            tracing_interceptor,
        ))
        .add_service(reflection_service)
        .serve_with_shutdown(listen_addr, async move {
            let _ = shutdown_rx.recv().await;
            info!("gRPC server received shutdown signal");
        })
        .await;

    match serve_result {
        Ok(()) => {
            info!("gRPC server stopped gracefully");
            Ok(())
        }
        Err(e) => {
            error!(error = ?e, "gRPC server failed");
            Err(e.into())
        }
    }
}
