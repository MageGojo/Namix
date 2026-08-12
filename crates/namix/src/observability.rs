//! Request IDs, structured spans, and database timing hooks.

use crate::{MiddlewareFn, Next, Request, wrap_middleware};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tracing::Instrument as _;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RequestId(pub String);

impl RequestId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub fn request_id_middleware() -> MiddlewareFn {
    wrap_middleware(|mut req: Request, next: Next| async move {
        let id = req
            .header("x-request-id")
            .filter(|value| valid_id(value))
            .map(str::to_owned)
            .unwrap_or_else(new_request_id);
        req.set(RequestId(id.clone()));
        let method = req.method().to_string();
        let path = req.path().to_owned();
        let started = Instant::now();
        let span = tracing::info_span!(
            "http.server.request",
            request_id = %id,
            http_method = %method,
            http_path = %path,
            http_status = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
        );
        let response = next.run(req).instrument(span.clone()).await;
        let status = response.status().as_u16();
        let duration_ms = started.elapsed().as_millis() as u64;
        span.record("http_status", status);
        span.record("duration_ms", duration_ms);
        tracing::info!(parent: &span, %status, duration_ms, "http request");
        response.with_header("x-request-id", id)
    })
}
pub fn request_id(req: &Request) -> Option<&str> {
    req.get::<RequestId>().map(|id| id.0.as_str())
}
pub fn measure_db<T>(operation: &str, run: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = run();
    record_db(operation, started, false);
    value
}

/// Time a fallible database operation and include its outcome without
/// inspecting or stringifying the application's error type.
pub fn measure_db_result<T, E>(
    operation: &str,
    run: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let started = Instant::now();
    let result = run();
    record_db(operation, started, result.is_err());
    result
}

pub async fn measure_db_async<T>(operation: &str, run: impl std::future::Future<Output = T>) -> T {
    let started = Instant::now();
    let value = run.await;
    record_db(operation, started, false);
    value
}

pub async fn measure_db_result_async<T, E>(
    operation: &str,
    run: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let started = Instant::now();
    let result = run.await;
    record_db(operation, started, result.is_err());
    result
}

fn record_db(operation: &str, started: Instant, failed: bool) {
    tracing::debug!(
        db_operation = operation,
        duration_ms = started.elapsed().as_millis(),
        error = failed,
        "database operation"
    );
}

fn new_request_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!(
        "nx-{:x}-{:x}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_id_validation_rejects_header_injection() {
        assert!(valid_id("trace_123.example"));
        assert!(!valid_id("trace\r\nx-forged: yes"));
        assert!(!valid_id(""));
        assert!(!valid_id(&"x".repeat(129)));
    }

    #[test]
    fn generated_request_ids_are_unique_and_valid() {
        let first = new_request_id();
        let second = new_request_id();
        assert_ne!(first, second);
        assert!(valid_id(&first));
        assert_eq!(RequestId(first.clone()).to_string(), first);
    }

    #[test]
    fn result_measurement_preserves_the_original_result() {
        let result: Result<u64, &'static str> = measure_db_result("select", || Err("db down"));
        assert_eq!(result, Err("db down"));
    }
}
