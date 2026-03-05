# Tracing Configuration

The IBC Attestor supports OpenTelemetry Protocol (OTLP) trace export to backends such as Grafana Tempo, Jaeger, or any OTLP collector.

## Logging Format

All logs are emitted in JSON format with RFC 3339 UTC timestamps. When tracing is initialized, logs automatically include `trace_id` and `span_id` fields for correlation with distributed traces. This correlation works regardless of whether OTLP export is enabled.

The `RUST_LOG` environment variable controls log level (defaults to `info`).

## Configuration

The `[tracing]` section is all-or-nothing: either provide a complete section with all three fields, or omit it entirely.

| Field | Type | Description |
|-------|------|-------------|
| `otlp_endpoint` | URL | OTLP gRPC endpoint (e.g., `http://tempo:4317`) |
| `service_name` | String | Service name for traces |
| `sample_rate` | Float | Sampling ratio (see below) |

### Sample Rate Behavior

- Values in `[0.0, 1.0)`: Uses `TraceIdRatioBased` sampling (e.g., `0.1` samples 10% of traces)
- Values `>= 1.0`: Uses `AlwaysOn` sampling (all traces captured)

### Sample Rate Recommendations

| Environment | Recommended Sample Rate |
|-------------|------------------------|
| Development | `1.0` (all traces) |
| Staging | `0.5` (50% of traces) |
| Production | `0.1` to `0.01` (1-10% of traces) |

## With OTLP Export

When the `[tracing]` section is present, traces are exported to the configured OTLP endpoint. W3C Trace Context propagation is enabled for distributed tracing across services.

Example configuration:

```toml
[tracing]
otlp_endpoint = "http://tempo:4317"
service_name = "ibc-attestor-prod"
sample_rate = 1.0
```

For namespaced Kubernetes deployments:

```toml
[tracing]
otlp_endpoint = "http://tempo.observability.svc.cluster.local:4317"
service_name = "ibc-attestor"
sample_rate = 0.1
```

## Without OTLP Export

When the `[tracing]` section is omitted, OTLP export is disabled but local tracing still runs. Logs continue to include `trace_id` and `span_id` fields, allowing log correlation without external trace infrastructure.

## TracingGuard Lifecycle

The `init_logging` function returns a `TracingGuard` that must be held for the application's lifetime. When the guard is dropped:

1. Pending spans are flushed to the OTLP endpoint
2. The tracer provider shuts down gracefully

Ensure the guard remains in scope until application shutdown to avoid losing trace data.
