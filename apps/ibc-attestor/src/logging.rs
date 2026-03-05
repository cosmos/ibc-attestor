use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,
    trace::{Sampler, SdkTracerProvider},
};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    fmt,
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::config::TracingConfig;

const DEFAULT_SERVICE_NAME: &str = "ibc-attestor";

/// Initialize structured logging with optional OTLP trace export.
///
/// Sets up tracing-subscriber with:
/// - JSON formatting with RFC 3339 UTC timestamps
/// - OpenTelemetry layer for `trace_id` and `span_id` in logs
/// - Environment variable configuration via `RUST_LOG` (defaults to "info")
/// - W3C Trace Context propagation for distributed tracing
/// - OTLP trace export when config is provided
///
/// Returns a [`TracingGuard`] that must be held for the lifetime of the application.
/// When dropped, it flushes any pending spans to the OTLP endpoint.
///
/// Panics when an invalid [`TracingGuard`] is provided.
#[must_use]
pub fn init_logging(config: Option<TracingConfig>) -> TracingGuard {
    let (provider, service) = match config {
        Some(cfg) => (build_exporter_tracer(&cfg), cfg.service_name.clone()),
        None => (
            SdkTracerProvider::builder().build(),
            DEFAULT_SERVICE_NAME.to_string(),
        ),
    };
    let tracer = provider.tracer(service);
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    let otel_layer = OpenTelemetryLayer::new(tracer);

    let fmt_layer = fmt::layer()
        .json()
        .with_timer(fmt::time::UtcTime::rfc_3339())
        .with_current_span(false)
        .with_thread_ids(false)
        .with_line_number(true)
        .with_file(true)
        .with_target(false)
        .flatten_event(true)
        .with_ansi(false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(otel_layer)
        .with(fmt_layer)
        .init();

    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    TracingGuard { provider }
}

fn build_exporter_tracer(config: &TracingConfig) -> SdkTracerProvider {
    let sampler = if config.sample_rate == 1.0 {
        Sampler::AlwaysOn
    } else {
        Sampler::TraceIdRatioBased(config.sample_rate)
    };

    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(config.otlp_endpoint.as_str())
        .build()
        .expect("failed to create OTLP exporter");

    SdkTracerProvider::builder()
        .with_sampler(sampler)
        .with_batch_exporter(exporter)
        .build()
}

/// Guard that ensures the tracer provider is properly shut down.
///
/// When dropped, this guard flushes any pending spans to the OTLP endpoint
/// and shuts down the tracer provider gracefully.
pub struct TracingGuard {
    provider: SdkTracerProvider,
}

impl Drop for TracingGuard {
    fn drop(&mut self) {
        if let Err(e) = self.provider.shutdown() {
            eprintln!("Error shutting down tracer provider: {e}");
        }
    }
}
