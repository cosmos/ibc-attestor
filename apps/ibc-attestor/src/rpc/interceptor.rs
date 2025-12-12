use tonic::{metadata::MetadataMap, Request, Status};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Extractor for gRPC metadata that implements OpenTelemetry's Extractor trait
struct MetadataExtractor<'a>(&'a MetadataMap);

impl<'a> opentelemetry::propagation::Extractor for MetadataExtractor<'a> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .map(|k| match k {
                tonic::metadata::KeyRef::Ascii(key) => key.as_str(),
                tonic::metadata::KeyRef::Binary(key) => key.as_str(),
            })
            .collect()
    }
}

/// gRPC interceptor that automatically extracts trace context from incoming requests
///
/// This interceptor:
/// 1. Extracts parent trace context from gRPC metadata (if present)
/// 2. Sets it as the parent for the current span context
///
/// The OpenTelemetry trace context propagates to all spans created during the request.
/// The trace_id and span_id appear in JSON logs via the OpenTelemetryLayer configured
/// in `logging::init_logging()`.
///
/// Usage: Apply to gRPC server using `with_interceptor(service, tracing_interceptor)`
#[allow(clippy::result_large_err)] // Otherwise everyting needs wrapping as `Box`
pub fn tracing_interceptor<T>(request: Request<T>) -> Result<Request<T>, Status> {
    // Extract parent context from gRPC metadata using the configured propagator
    let parent_context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&MetadataExtractor(request.metadata()))
    });

    // Set the parent context. This will be inherited by all spans created during this request.
    // The service method's #[instrument] span will be a child of this context.
    let _ = Span::current().set_parent(parent_context);

    Ok(request)
}
