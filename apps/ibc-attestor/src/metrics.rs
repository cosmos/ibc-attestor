//! Prometheus metrics for the attestor.
//!
//! All metrics are registered against a process-wide [`prometheus::Registry`]
//! that carries two constant labels (`adapter`, `signer`) populated by
//! [`init`] at startup. Recording functions are no-ops when [`init`] has not
//! been called, which keeps unit tests in unrelated modules from having to
//! initialize the registry.

use std::collections::HashMap;
use std::future::Future;
use std::sync::OnceLock;
use std::time::Instant;

use prometheus::{
    Encoder, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts,
    Registry, TextEncoder,
};

static METRICS: OnceLock<Metrics> = OnceLock::new();

/// All metric handles plus the underlying registry.
struct Metrics {
    registry: Registry,
    rpc_requests_total: IntCounterVec,
    rpc_request_duration_seconds: HistogramVec,
    attestation_height_lag_blocks: Histogram,
    height_validation_rejections_total: IntCounter,
    commitment_validation_failures_total: IntCounterVec,
    adapter_retry_failures_total: IntCounterVec,
    adapter_finalized_height: IntGauge,
    signer_signs_total: IntCounterVec,
}

impl Metrics {
    #[allow(clippy::too_many_lines)]
    fn new(adapter: &'static str, signer: &'static str) -> Self {
        let mut const_labels = HashMap::new();
        const_labels.insert("adapter".to_string(), adapter.to_string());
        const_labels.insert("signer".to_string(), signer.to_string());

        let registry = Registry::new_custom(None, Some(const_labels))
            .expect("registry construction with valid labels never fails");

        let rpc_requests_total = IntCounterVec::new(
            Opts::new(
                "attestor_rpc_requests_total",
                "Total RPC requests by method and gRPC status code",
            ),
            &["method", "code"],
        )
        .expect("static metric definition is valid");
        registry
            .register(Box::new(rpc_requests_total.clone()))
            .expect("metric registration is unique at startup");

        let rpc_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "attestor_rpc_request_duration_seconds",
                "RPC request duration in seconds",
            ),
            &["method"],
        )
        .expect("static metric definition is valid");
        registry
            .register(Box::new(rpc_request_duration_seconds.clone()))
            .expect("metric registration is unique at startup");

        let attestation_height_lag_blocks = Histogram::with_opts(
            HistogramOpts::new(
                "attestor_attestation_height_lag_blocks",
                "finalized_height minus requested_height; positive = caller behind, negative = caller ahead",
            )
            .buckets(vec![
                -100.0, -10.0, -3.0, -1.0, 0.0, 1.0, 3.0, 10.0, 50.0, 100.0, 500.0,
            ]),
        )
        .expect("static metric definition is valid");
        registry
            .register(Box::new(attestation_height_lag_blocks.clone()))
            .expect("metric registration is unique at startup");

        let height_validation_rejections_total = IntCounter::new(
            "attestor_height_validation_rejections_total",
            "Number of attestation requests rejected because the requested height was beyond the latest finalized height",
        )
        .expect("static metric definition is valid");
        registry
            .register(Box::new(height_validation_rejections_total.clone()))
            .expect("metric registration is unique at startup");

        let commitment_validation_failures_total = IntCounterVec::new(
            Opts::new(
                "attestor_commitment_validation_failures_total",
                "Number of packet/ack/receipt commitment validation failures by kind",
            ),
            &["kind"],
        )
        .expect("static metric definition is valid");
        registry
            .register(Box::new(commitment_validation_failures_total.clone()))
            .expect("metric registration is unique at startup");

        let adapter_retry_failures_total = IntCounterVec::new(
            Opts::new(
                "attestor_adapter_retry_failures_total",
                "Number of adapter operations that exhausted retries without succeeding",
            ),
            &["op"],
        )
        .expect("static metric definition is valid");
        registry
            .register(Box::new(adapter_retry_failures_total.clone()))
            .expect("metric registration is unique at startup");

        let adapter_finalized_height = IntGauge::new(
            "attestor_adapter_finalized_height",
            "Latest finalized height returned by the chain adapter",
        )
        .expect("static metric definition is valid");
        registry
            .register(Box::new(adapter_finalized_height.clone()))
            .expect("metric registration is unique at startup");

        let signer_signs_total = IntCounterVec::new(
            Opts::new(
                "attestor_signer_signs_total",
                "Total signing operations attempted, by outcome",
            ),
            &["result"],
        )
        .expect("static metric definition is valid");
        registry
            .register(Box::new(signer_signs_total.clone()))
            .expect("metric registration is unique at startup");

        Self {
            registry,
            rpc_requests_total,
            rpc_request_duration_seconds,
            attestation_height_lag_blocks,
            height_validation_rejections_total,
            commitment_validation_failures_total,
            adapter_retry_failures_total,
            adapter_finalized_height,
            signer_signs_total,
        }
    }
}

fn metrics() -> Option<&'static Metrics> {
    METRICS.get()
}

/// Initialize the global metrics registry with constant labels for the
/// configured `adapter` and `signer`. Idempotent — subsequent calls are
/// silently ignored.
pub fn init(adapter: &'static str, signer: &'static str) {
    let _ = METRICS.set(Metrics::new(adapter, signer));
}

/// Record a finished RPC: counter and latency histogram in one place to
/// keep the call site small.
pub fn record_rpc_request(method: &str, code: &str, duration_seconds: f64) {
    if let Some(m) = metrics() {
        m.rpc_requests_total
            .with_label_values(&[method, code])
            .inc();
        m.rpc_request_duration_seconds
            .with_label_values(&[method])
            .observe(duration_seconds);
    }
}

/// Observe `finalized_height − requested_height` for an attestation request.
pub fn observe_height_lag(lag_blocks: i64) {
    if let Some(m) = metrics() {
        #[allow(clippy::cast_precision_loss)]
        m.attestation_height_lag_blocks.observe(lag_blocks as f64);
    }
}

/// Increment when a request is rejected because the requested height is
/// beyond the latest finalized height.
pub fn inc_height_rejections() {
    if let Some(m) = metrics() {
        m.height_validation_rejections_total.inc();
    }
}

/// Increment when a commitment validation fails. `kind` should be one of
/// `not_found`, `mismatch`, or `unexpected_receipt`.
pub fn inc_commitment_failure(kind: &str) {
    if let Some(m) = metrics() {
        m.commitment_validation_failures_total
            .with_label_values(&[kind])
            .inc();
    }
}

/// Increment when a retried adapter operation hits `MAX_ATTEMPTS` without
/// succeeding.
pub fn inc_retry_failure(op: &str) {
    if let Some(m) = metrics() {
        m.adapter_retry_failures_total
            .with_label_values(&[op])
            .inc();
    }
}

/// Set the gauge that tracks the latest finalized block height returned by
/// the configured adapter.
pub fn set_adapter_finalized_height(height: u64) {
    if let Some(m) = metrics() {
        let value = i64::try_from(height).unwrap_or(i64::MAX);
        m.adapter_finalized_height.set(value);
    }
}

/// Increment for each signer call. `result` ∈ {`ok`, `err`}.
pub fn inc_signer_sign(result: &str) {
    if let Some(m) = metrics() {
        m.signer_signs_total.with_label_values(&[result]).inc();
    }
}

/// Encode the gathered metrics in Prometheus text format. Returns an empty
/// buffer if the registry has not been initialized.
#[must_use]
pub fn encode_text() -> Vec<u8> {
    let Some(m) = metrics() else {
        return Vec::new();
    };
    let mut buf = Vec::new();
    let encoder = TextEncoder::new();
    let _ = encoder.encode(&m.registry.gather(), &mut buf);
    buf
}

/// The Prometheus content type for the text exposition format.
pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4";

/// Record `attestor_rpc_requests_total` and `attestor_rpc_request_duration_seconds`
/// for the awaited gRPC handler future.
///
/// # Errors
/// Forwards the inner future's `Err(tonic::Status)` unchanged.
pub async fn track_rpc<F, T>(method: &str, fut: F) -> Result<T, tonic::Status>
where
    F: Future<Output = Result<T, tonic::Status>>,
{
    let start = Instant::now();
    let result = fut.await;
    let elapsed = start.elapsed().as_secs_f64();
    let code = match &result {
        Ok(_) => tonic::Code::Ok,
        Err(status) => status.code(),
    };
    record_rpc_request(method, &code_label(code), elapsed);
    result
}

/// Convert a [`tonic::Code`] into a label value.
#[must_use]
pub fn code_label(code: tonic::Code) -> String {
    pascal_to_snake(&format!("{code:?}"))
}

fn pascal_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, b) in s.bytes().enumerate() {
        if b.is_ascii_uppercase() && i != 0 {
            out.push('_');
        }
        out.push(b.to_ascii_lowercase() as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_register_and_export_with_constant_labels() {
        init("test_adapter", "test_signer");

        record_rpc_request("latest_height", "ok", 0.05);
        observe_height_lag(-1);
        observe_height_lag(5);
        inc_height_rejections();
        inc_commitment_failure("not_found");
        inc_commitment_failure("mismatch");
        inc_retry_failure("evm.latest_height");
        set_adapter_finalized_height(123);
        inc_signer_sign("ok");

        let body = encode_text();
        let exposed = String::from_utf8(body).expect("text format is utf-8");

        for name in [
            "attestor_rpc_requests_total",
            "attestor_rpc_request_duration_seconds",
            "attestor_attestation_height_lag_blocks",
            "attestor_height_validation_rejections_total",
            "attestor_commitment_validation_failures_total",
            "attestor_adapter_retry_failures_total",
            "attestor_adapter_finalized_height",
            "attestor_signer_signs_total",
        ] {
            assert!(
                exposed.contains(name),
                "expected `{name}` in exposed metrics; got:\n{exposed}"
            );
        }
        // The init call may have lost a race with another test, but constant
        // labels still must be present whichever init won.
        assert!(exposed.contains("adapter=\""));
        assert!(exposed.contains("signer=\""));

        // Negative bucket exists for height lag.
        assert!(exposed.contains("attestor_attestation_height_lag_blocks_bucket"));
        assert!(exposed.contains("le=\"-1\""));
    }

    #[test]
    fn code_label_covers_common_codes() {
        assert_eq!(code_label(tonic::Code::Ok), "ok");
        assert_eq!(code_label(tonic::Code::Internal), "internal");
        assert_eq!(
            code_label(tonic::Code::FailedPrecondition),
            "failed_precondition"
        );
        assert_eq!(code_label(tonic::Code::NotFound), "not_found");
    }

    #[test]
    fn pascal_to_snake_handles_typical_inputs() {
        assert_eq!(pascal_to_snake("LatestHeight"), "latest_height");
        assert_eq!(
            pascal_to_snake("ServerReflectionInfo"),
            "server_reflection_info"
        );
        assert_eq!(pascal_to_snake("Ok"), "ok");
    }
}
