//! HTTP health check server for Kubernetes readiness probes.

use std::net::SocketAddr;

use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tracing::{debug, info};
use warp::Filter;

/// Start the HTTP health server.
///
/// Serves a `GET /healthz` endpoint that verifies the gRPC server is accepting
/// connections before returning HTTP 200. Returns HTTP 503 if the gRPC server
/// is not reachable.
///
/// This ensures the health check only passes once the gRPC server is ready.
#[tracing::instrument(skip_all, fields(health_addr = %addr, grpc_addr = %grpc_addr))]
pub async fn start(
    addr: SocketAddr,
    grpc_addr: SocketAddr,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let healthz = warp::get()
        .and(warp::path!("healthz"))
        .then(move || check_grpc_ready(grpc_addr));

    info!(health_addr = %addr, grpc_addr = %grpc_addr, "health server ready, listening for requests");

    warp::serve(healthz)
        .bind(addr)
        .await
        .graceful(async move {
            let _ = shutdown_rx.recv().await;
            info!("health server received shutdown signal");
        })
        .run()
        .await;

    info!("health server stopped gracefully");
}

async fn check_grpc_ready(grpc_addr: SocketAddr) -> warp::http::StatusCode {
    match TcpStream::connect(grpc_addr).await {
        Ok(_) => {
            debug!("gRPC server is ready");
            warp::http::StatusCode::OK
        }
        Err(_) => {
            debug!("gRPC server is not ready");
            warp::http::StatusCode::SERVICE_UNAVAILABLE
        }
    }
}
