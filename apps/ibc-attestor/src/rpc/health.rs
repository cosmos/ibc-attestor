use std::net::SocketAddr;

use tokio::{net::TcpStream, sync::broadcast};
use tracing::{error, info};
use warp::{Filter, Reply, http::StatusCode};

use crate::metrics;

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

fn make_metrics_filter()
-> impl Filter<Extract = (warp::reply::WithHeader<Vec<u8>>,), Error = warp::Rejection> + Clone + Send
{
    warp::get()
        .and(warp::path("metrics"))
        .and(warp::path::end())
        .map(|| {
            warp::reply::with_header(
                metrics::encode_text(),
                "content-type",
                metrics::CONTENT_TYPE,
            )
        })
}

/// Start the HTTP health server.
///
/// Exposes a `GET /healthz` endpoint that returns 200 OK when the gRPC server
/// is accepting connections, or 503 Service Unavailable when it is not ready,
/// and a `GET /metrics` endpoint that returns the current Prometheus metrics.
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

    let healthz = make_healthz_filter(grpc_addr).map(Reply::into_response);
    let metrics_route = make_metrics_filter().map(Reply::into_response);
    let routes = healthz.or(metrics_route);

    let shutdown_signal = async move {
        let _ = shutdown_rx.recv().await;
        info!("health server received shutdown signal");
    };

    warp::serve(routes)
        .bind(health_addr)
        .await
        .graceful(shutdown_signal)
        .run()
        .await;

    info!("health server stopped gracefully");
}
