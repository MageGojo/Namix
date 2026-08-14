//! 可选的 HTML 错误页（404 / 403 / 500 / 429 …）。
//!
//! 不注册时保持框架默认：浏览器走 [`AppError`](super::error::AppError) 的通用 HTML，
//! JSON 与 `#[server]` 始终是 `{ error, message, errors }`。
//!
//! ```ignore
//! routes! { /* … */ }
//!     .error_page(404, errors::page)   // 只覆盖某一个状态
//!     .error_pages(errors::page)       // 其余 HTML 错误共用一页
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use http::StatusCode;

use super::request::Request;
use super::response::Response;

/// 错误页渲染器收到的上下文。
#[derive(Clone, Debug)]
pub struct ErrorPage {
    pub status: u16,
    pub message: String,
}

impl ErrorPage {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// HTTP 标准短语，如 `Not Found`。应用可再映射成中文标题。
    pub fn reason(&self) -> &'static str {
        StatusCode::from_u16(self.status)
            .ok()
            .and_then(|status| status.canonical_reason())
            .unwrap_or("Error")
    }
}

pub type ErrorPageFn = Arc<dyn Fn(&Request, ErrorPage) -> Response + Send + Sync>;

#[derive(Clone, Default)]
struct Inner {
    by_status: HashMap<u16, ErrorPageFn>,
    any: Option<ErrorPageFn>,
}

/// 按状态码注册的 HTML 错误页表。挂在 [`Router`](crate::Router) 上，dispatch 时写入 `Request`。
#[derive(Clone, Default)]
pub struct ErrorPages {
    inner: Arc<Inner>,
}

impl ErrorPages {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.by_status.is_empty() && self.inner.any.is_none()
    }

    /// 某一个状态码（后写覆盖先写）。
    pub fn page(
        self,
        status: u16,
        render: impl Fn(&Request, ErrorPage) -> Response + Send + Sync + 'static,
    ) -> Self {
        let mut inner = (*self.inner).clone();
        inner.by_status.insert(status, Arc::new(render));
        Self {
            inner: Arc::new(inner),
        }
    }

    /// 未单独注册的状态都走这里。具体状态优先。
    pub fn any(
        self,
        render: impl Fn(&Request, ErrorPage) -> Response + Send + Sync + 'static,
    ) -> Self {
        let mut inner = (*self.inner).clone();
        inner.any = Some(Arc::new(render));
        Self {
            inner: Arc::new(inner),
        }
    }

    /// `other` 覆盖相同状态码和 catch-all。
    pub fn merge(self, other: Self) -> Self {
        if other.is_empty() {
            return self;
        }
        if self.is_empty() {
            return other;
        }
        let mut inner = (*self.inner).clone();
        for (status, render) in &other.inner.by_status {
            inner.by_status.insert(*status, render.clone());
        }
        if other.inner.any.is_some() {
            inner.any = other.inner.any.clone();
        }
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn render(&self, req: &Request, status: u16, message: &str) -> Option<Response> {
        let render = self
            .inner
            .by_status
            .get(&status)
            .or(self.inner.any.as_ref())?;
        let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        Some(
            render(
                req,
                ErrorPage {
                    status,
                    message: message.to_string(),
                },
            )
            .with_status(status_code),
        )
    }

    /// 请求上若挂了错误页表，则渲染对应状态。
    pub fn try_render(req: &Request, status: u16, message: &str) -> Option<Response> {
        req.get::<ErrorPages>()?.render(req, status, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::controller::html;
    use crate::core::error::AppError;
    use crate::core::routing::Router;
    use crate::core::test_client::TestClient;

    fn branded(_req: &Request, error: ErrorPage) -> Response {
        html(format!("branded-{}-{}", error.status, error.message))
    }

    fn only_404(_req: &Request, error: ErrorPage) -> Response {
        html(format!("four-oh-four:{}", error.status))
    }

    async fn missing(_req: crate::core::request::Request) -> Result<Response, AppError> {
        Err(AppError::NotFound)
    }

    async fn denied(_req: crate::core::request::Request) -> Result<Response, AppError> {
        Err(AppError::Forbidden)
    }

    async fn limited(_req: crate::core::request::Request) -> Result<Response, AppError> {
        Err(AppError::RateLimited { retry_after: 9 })
    }

    #[tokio::test]
    async fn unmatched_and_app_error_use_registered_html_page() {
        let router = Router::new().get("/gone", missing).error_page(404, branded);
        let mut client = TestClient::new(router);

        let unmatched = client.get("/nope").await;
        assert_eq!(unmatched.status, StatusCode::NOT_FOUND);
        assert!(unmatched.text().contains("branded-404-not found"));

        let gone = client.get("/gone").await;
        assert_eq!(gone.status, StatusCode::NOT_FOUND);
        assert!(gone.text().contains("branded-404-not found"));
    }

    #[tokio::test]
    async fn specific_status_wins_over_catch_all() {
        let router = Router::new()
            .get("/gone", missing)
            .get("/nope-auth", denied)
            .error_page(404, only_404)
            .error_pages(branded);
        let mut client = TestClient::new(router);

        assert!(
            client
                .get("/missing")
                .await
                .text()
                .contains("four-oh-four:404")
        );
        assert!(
            client
                .get("/nope-auth")
                .await
                .text()
                .contains("branded-403-forbidden")
        );
    }

    #[tokio::test]
    async fn json_requests_ignore_html_error_pages() {
        let router = Router::new().error_pages(branded);
        let mut client = TestClient::new(router)
            .with_default_header("accept", "application/json")
            .unwrap();

        let response = client.get("/nope").await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert!(response.text().contains("\"error\":\"not found\""));
        assert!(!response.text().contains("branded-"));
    }

    #[tokio::test]
    async fn api_prefix_unmatched_stays_json() {
        let router = Router::new().error_page(404, branded);
        let mut client = TestClient::new(router);
        let response = client.get("/api/missing").await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert!(response.text().contains("\"message\":\"not found\""));
        assert!(!response.text().contains("branded-"));
    }

    #[tokio::test]
    async fn rate_limit_html_page_keeps_retry_after() {
        let router = Router::new().get("/slow", limited).error_pages(branded);
        let mut client = TestClient::new(router);
        let response = client.get("/slow").await;
        assert_eq!(response.status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.header("retry-after"), Some("9"));
        assert!(response.text().contains("branded-429-"));
    }

    #[tokio::test]
    async fn default_unmatched_html_without_custom_page() {
        let router = Router::new();
        let mut client = TestClient::new(router);
        let response = client.get("/nope").await;
        assert_eq!(response.status, StatusCode::NOT_FOUND);
        assert_eq!(
            response.header("content-type"),
            Some("text/html; charset=utf-8")
        );
        assert!(response.text().contains("<h1>Not Found</h1>"));
    }
}
