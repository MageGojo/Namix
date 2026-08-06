//! In-process HTTP test client for routes, cookies, forms, Actions, and WS routes.

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, Uri};

use super::middleware::MiddlewareFn;
use super::request::Request;
use super::response::Response;
use super::routing::Router;

#[derive(Clone)]
pub struct TestClient {
    router: Router,
    middlewares: Arc<Vec<MiddlewareFn>>,
    cookies: BTreeMap<String, String>,
}
#[derive(Clone, Debug)]
pub struct TestResponse {
    pub status: http::StatusCode,
    pub headers: HeaderMap,
    pub body: Bytes,
}
impl TestResponse {
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or("")
    }
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }
}
#[derive(Clone, Debug)]
pub struct TestWebSocket {
    pub path: String,
    pub connected: bool,
}

impl TestClient {
    pub fn new(router: Router) -> Self {
        Self {
            router,
            middlewares: Arc::new(Vec::new()),
            cookies: BTreeMap::new(),
        }
    }
    pub fn with_middleware(mut self, middleware: Vec<MiddlewareFn>) -> Self {
        self.middlewares = Arc::new(middleware);
        self
    }
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }
    pub async fn get(&mut self, uri: &str) -> TestResponse {
        self.request(Method::GET, uri, Bytes::new(), None).await
    }
    pub async fn form(
        &mut self,
        method: Method,
        uri: &str,
        values: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>,
    ) -> TestResponse {
        let body = values
            .into_iter()
            .map(|(k, v)| format!("{}={}", url_encode(k.as_ref()), url_encode(v.as_ref())))
            .collect::<Vec<_>>()
            .join("&");
        self.request(
            method,
            uri,
            Bytes::from(body),
            Some("application/x-www-form-urlencoded"),
        )
        .await
    }
    pub async fn json(
        &mut self,
        method: Method,
        uri: &str,
        value: &impl serde::Serialize,
    ) -> TestResponse {
        self.request(
            method,
            uri,
            Bytes::from(serde_json::to_vec(value).expect("test json")),
            Some("application/json"),
        )
        .await
    }
    pub async fn action(&mut self, token: &str, input: impl serde::Serialize) -> TestResponse {
        let body = serde_json::json!({"t":token,"i":input,"ts": now_secs()});
        self.json(Method::POST, "/api/a", &body).await
    }
    pub async fn websocket(&self, path: &str) -> TestWebSocket {
        TestWebSocket {
            path: path.into(),
            connected: self.router.match_ws(path).is_some(),
        }
    }
    pub async fn request(
        &mut self,
        method: Method,
        uri: &str,
        body: Bytes,
        content_type: Option<&str>,
    ) -> TestResponse {
        let mut headers = HeaderMap::new();
        if let Some(content_type) = content_type {
            headers.insert("content-type", HeaderValue::from_str(content_type).unwrap());
        }
        if !self.cookies.is_empty() {
            let cookie = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            headers.insert("cookie", HeaderValue::from_str(&cookie).unwrap());
        }
        let uri: Uri = uri.parse().expect("test request uri");
        let request = Request::new(method, uri, headers, body);
        let response = self
            .router
            .dispatch(request, Arc::clone(&self.middlewares))
            .await;
        self.capture_cookies(&response);
        let (status, headers, body) = response.into_status_headers_body().await;
        TestResponse {
            status,
            headers,
            body,
        }
    }
    fn capture_cookies(&mut self, response: &Response) {
        for value in response.headers().get_all("set-cookie") {
            let Ok(raw) = value.to_str() else { continue };
            let Some((name, value)) = raw
                .split(';')
                .next()
                .and_then(|first| first.split_once('='))
            else {
                continue;
            };
            if raw.contains("Max-Age=0") {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.into(), value.into());
            }
        }
    }
}
fn url_encode(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            b' ' => vec!['+'],
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{controller::text, routing::Route};
    async fn hello(_: Request) -> Response {
        text("hello")
    }
    #[tokio::test]
    async fn visits_route_and_keeps_cookie_jar() {
        let router = Route::get("/", hello).register();
        let mut client = TestClient::new(router);
        assert_eq!(client.get("/").await.text(), "hello");
    }
}
