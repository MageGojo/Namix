//! Request IDs, structured spans, and database timing hooks.

use crate::{MiddlewareFn, Next, Request, wrap_middleware};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct RequestId(pub String);
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
        let response = next.run(req).await;
        tracing::info!(request_id=%id,%method,%path,status=response.status().as_u16(),duration_ms=started.elapsed().as_millis(),"http request");
        response.with_header("x-request-id", id)
    })
}
pub fn request_id(req: &Request) -> Option<&str> {
    req.get::<RequestId>().map(|id| id.0.as_str())
}
pub fn measure_db<T>(operation: &str, run: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = run();
    tracing::debug!(
        db_operation = operation,
        duration_ms = started.elapsed().as_millis(),
        "database operation"
    );
    value
}
pub async fn measure_db_async<T>(operation: &str, run: impl std::future::Future<Output = T>) -> T {
    let started = Instant::now();
    let value = run.await;
    tracing::debug!(
        db_operation = operation,
        duration_ms = started.elapsed().as_millis(),
        "database operation"
    );
    value
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
