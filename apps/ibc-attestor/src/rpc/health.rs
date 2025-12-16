use tonic::{Request, Response, Status};
use tracing::debug;

use crate::rpc::health_api::health_server::Health;
use crate::rpc::health_api::{HealthCheckRequest, HealthCheckResponse};

/// Health check service implementation
#[derive(Debug, Clone)]
pub struct HealthService;

impl HealthService {
    /// Creates a new health service
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for HealthService {
    fn default() -> Self {
        Self::new()
    }
}

type WatchStream = futures::stream::Empty<Result<HealthCheckResponse, Status>>;

#[tonic::async_trait]
impl Health for HealthService {
    type WatchStream = WatchStream;

    async fn check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let service_name = &request.get_ref().service;
        debug!(service = service_name, "health check request received");

        // Always return SERVING - if we're responding, we're healthy
        let response = HealthCheckResponse {
            status: 1, // SERVING
        };

        Ok(Response::new(response))
    }

    async fn watch(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<Self::WatchStream>, Status> {
        // Watch is not implemented for this simple health check
        Err(Status::unimplemented("watch is not implemented"))
    }
}
