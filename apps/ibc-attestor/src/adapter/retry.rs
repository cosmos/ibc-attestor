use std::{
    future::Future,
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};
use tokio_retry::{RetryIf, strategy::ExponentialBackoff};
use tracing::{debug, error};

use super::AttestationAdapterError;
use crate::metrics;

const MAX_ATTEMPTS: u8 = 3;
const INITIAL_BACKOFF: u64 = 200;
const MAX_BACKOFF: Duration = Duration::from_secs(2);

pub(super) async fn with_retry_backoff<T, F, Fut>(
    operation: &'static str,
    mut request: F,
) -> Result<T, AttestationAdapterError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, AttestationAdapterError>>,
{
    let retry_strategy = ExponentialBackoff::from_millis(INITIAL_BACKOFF)
        .factor(2)
        .max_delay(MAX_BACKOFF)
        .take(usize::from(MAX_ATTEMPTS.saturating_sub(1)));

    let attempts = AtomicU8::new(0);
    let result = RetryIf::spawn(
        retry_strategy,
        || {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            debug!(
                operation,
                attempt,
                maxAttempts = MAX_ATTEMPTS,
                "request attempt"
            );
            request()
        },
        |error: &AttestationAdapterError| {
            matches!(error, AttestationAdapterError::RetrievalError(_))
        },
    )
    .await;

    let final_attempts = attempts.load(Ordering::Relaxed);

    if let Err(error) = &result {
        if final_attempts >= MAX_ATTEMPTS {
            metrics::inc_retry_failure(operation);
        }
        error!(
            operation,
            attempts = final_attempts,
            maxAttempts = MAX_ATTEMPTS,
            error = %error,
            "request failed"
        );
    }

    result
}
