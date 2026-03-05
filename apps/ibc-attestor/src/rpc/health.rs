use std::net::SocketAddr;

use tokio::{net::TcpStream, sync::broadcast};
use tracing::{error, info};
use warp::{Filter, http::StatusCode};

async fn check_grpc(grpc_addr: SocketAddr) -> StatusCode {
    match TcpStream::connect(grpc_addr).await {
        Ok(_) => {
            info!("health check passed: gRPC server is accepting connections");
            StatusCode::OK
        }
        Err(e) => {
            error!(error = %e, "health check failed: gRPC server not ready");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

fn make_healthz_filter(
    grpc_addr: SocketAddr,
) -> impl Filter<Extract = (StatusCode,), Error = warp::Rejection> + Clone + Send {
    warp::get()
        .and(warp::path("healthz"))
        .and(warp::path::end())
        .map(move || grpc_addr)
        .then(check_grpc)
}

/// Start the HTTP health server.
///
/// Exposes a `GET /healthz` endpoint that returns 200 OK when the gRPC server
/// is accepting connections, or 503 Service Unavailable when it is not ready.
pub async fn start(
    health_addr: SocketAddr,
    grpc_addr: SocketAddr,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    info!(
        health_addr = %health_addr,
        grpc_addr = %grpc_addr,
        "starting HTTP health server"
    );

    let healthz = make_healthz_filter(grpc_addr);

    let shutdown_signal = async move {
        let _ = shutdown_rx.recv().await;
        info!("health server received shutdown signal");
    };

    warp::serve(healthz)
        .bind(health_addr)
        .await
        .graceful(shutdown_signal)
        .run()
        .await;

    info!("health server stopped gracefully");
}
