use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    fmt,
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

/// Initialize structured logging with OpenTelemetry integration
///
/// Sets up tracing-subscriber with:
/// - JSON formatting with RFC 3339 UTC timestamps
/// - OpenTelemetry layer for trace_id and span_id in logs
/// - Environment variable configuration via RUST_LOG (defaults to "info")
/// - W3C Trace Context propagation for distributed tracing
pub fn init_logging() {
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    // Create an OpenTelemetry tracer for trace_id and span_id injection
    let provider = SdkTracerProvider::builder().build();
    let tracer = provider.tracer("ibc-attestor");

    // OpenTelemetry layer adds trace_id and span_id to all spans
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
}
