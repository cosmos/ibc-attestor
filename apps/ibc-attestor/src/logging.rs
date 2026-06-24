use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    propagation::TraceContextPropagator,
    trace::{Sampler, SdkTracerProvider},
};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{
    filter::{EnvFilter, LevelFilter},
    fmt::{self, time::UtcTime},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::config::TracingConfig;

const DEFAULT_SERVICE_NAME: &str = "ibc-attestor";

/// Initialize JSON logging with optional OTLP trace export. In `plugin_mode` logs
/// go to stderr, since stdout carries the go-plugin handshake line.
///
/// The returned [`TracingGuard`] must be held for the process lifetime; on drop it
/// flushes pending spans.
#[must_use]
pub fn init_logging(config: Option<TracingConfig>, plugin_mode: bool) -> TracingGuard {
    let service = config.as_ref().map_or_else(
        || DEFAULT_SERVICE_NAME.to_string(),
        |c| c.service_name.clone(),
    );
    let provider = config.map_or_else(
        || {
            SdkTracerProvider::builder()
                .with_resource(Resource::builder().with_service_name(service.clone()).build())
                .build()
        },
        |cfg| build_exporter_tracer(&cfg, &service),
    );
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    let otel_layer = OpenTelemetryLayer::new(provider.tracer(service));
    let fmt_layer = fmt::layer()
        .json()
        .with_timer(UtcTime::rfc_3339())
        .with_current_span(false)
        .with_line_number(true)
        .with_file(true)
        .with_target(false)
        .flatten_event(true)
        .with_ansi(false);

    let subscriber = tracing_subscriber::registry().with(env_filter).with(otel_layer);
    if plugin_mode {
        subscriber.with(fmt_layer.with_writer(std::io::stderr)).init();
    } else {
        subscriber.with(fmt_layer).init();
    }

    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
    TracingGuard { provider }
}

fn build_exporter_tracer(config: &TracingConfig, service_name: &str) -> SdkTracerProvider {
    let sampler = if config.sample_rate.trunc() - 1.0 == 0.0 {
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
        .with_resource(Resource::builder().with_service_name(service_name.to_string()).build())
        .with_batch_exporter(exporter)
        .build()
}

/// Flushes and shuts down the tracer provider on drop.
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
