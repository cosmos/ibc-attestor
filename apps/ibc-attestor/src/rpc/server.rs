use std::net::SocketAddr;

use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic_health::ServingStatus;
use tracing::{error, info};

use super::{LoggingMiddleware, RpcError, attestor::AttestorService, tracing_interceptor};
use crate::adapter::AttestationAdapter;
use crate::rpc::api::FILE_DESCRIPTOR_SET;
use crate::rpc::api::attestation_service_server::AttestationServiceServer;
use crate::signer::Signer;

/// Well-known gRPC health service name that go-plugin polls to confirm liveness.
const GO_PLUGIN_HEALTH_SERVICE: &str = "plugin";

/// Start the gRPC server with attestation and reflection services.
///
/// # Errors
/// Returns [`RpcError::ServerError`] if the server fails to start or encounters
/// a fatal error during operation.
///
/// # Panics
/// Panics if the embedded protobuf file descriptor set is invalid. This should
/// never occur in practice as it's validated at compile time.
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

/// Start the gRPC server as a go-plugin plugin over an already-bound listener.
///
/// Like [`start`], plus the `grpc.health.v1.Health` service (`"plugin"` = SERVING)
/// that go-plugin polls. The caller binds `listener` and prints the handshake line.
///
/// # Errors
/// Returns [`RpcError::ServerError`] if the server encounters a fatal error.
///
/// # Panics
/// Panics if the embedded protobuf file descriptor set is invalid. This is
/// validated at compile time and is therefore infallible at runtime.
pub async fn start_plugin<A, S>(
    listener: TcpListener,
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
    let reflection_service = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build_v1()
        .expect("building reflection service should never fail with valid embedded descriptor set");

    let attestation_service = AttestorService::new(adapter, adapter_name, signer, signer_name);
    let logging_service = LoggingMiddleware::new(attestation_service);

    let (health_reporter, health_service) = tonic_health::server::health_reporter();
    health_reporter
        .set_service_status(GO_PLUGIN_HEALTH_SERVICE, ServingStatus::Serving)
        .await;

    info!(adapter = adapter_name, "gRPC plugin server ready, serving requests");

    let serve_result = Server::builder()
        .add_service(health_service)
        .add_service(AttestationServiceServer::with_interceptor(
            logging_service,
            tracing_interceptor,
        ))
        .add_service(reflection_service)
        .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async move {
            let _ = shutdown_rx.recv().await;
            info!("gRPC plugin server received shutdown signal");
        })
        .await;

    match serve_result {
        Ok(()) => {
            info!("gRPC plugin server stopped gracefully");
            Ok(())
        }
        Err(e) => {
            error!(error = ?e, "gRPC plugin server failed");
            Err(e.into())
        }
    }
}
