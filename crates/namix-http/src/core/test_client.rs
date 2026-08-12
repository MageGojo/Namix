//! In-process browser-style test client for routes, cookies, forms, Actions,
//! CSRF-protected mutations, and WebSocket route matching.

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use http::header::{CONTENT_TYPE, COOKIE, HOST, HeaderName, HeaderValue};
use http::{HeaderMap, Method, Uri};
use serde::Serialize;
use thiserror::Error;

use super::csrf::CsrfConfig;
use super::middleware::MiddlewareFn;
use super::request::Request;
use super::response::Response;
use super::routing::Router;

const DEFAULT_TEST_HOST: &str = "app.test";
const DEFAULT_TEST_ORIGIN: &str = "http://app.test";
const DEFAULT_CSRF_COOKIE: &str = "namix_csrf";
const DEFAULT_CSRF_HEADER: &str = "x-csrf-token";
const DEFAULT_CSRF_FIELD: &str = "_csrf";

pub type TestClientResult<T> = Result<T, TestClientError>;

/// Errors produced while constructing a test request or asserting an
/// in-process route match. Existing convenience methods such as [`TestClient::get`]
/// remain infallible at the type level; their `try_*` counterparts preserve
/// these errors for tests that need precise failure assertions.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TestClientError {
    #[error("invalid test request URI `{uri}`")]
    InvalidUri {
        uri: String,
        #[source]
        source: http::uri::InvalidUri,
    },
    #[error("invalid test header name `{name}`")]
    InvalidHeaderName {
        name: String,
        #[source]
        source: http::header::InvalidHeaderName,
    },
    #[error("invalid value for test header `{name}`")]
    InvalidHeaderValue {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("invalid same-origin URL `{origin}`; expected http(s)://host[:port]")]
    InvalidOrigin { origin: String },
    #[error("invalid test cookie `{name}`")]
    InvalidCookie { name: String },
    #[error("CSRF cookie `{name}` is missing; fetch a CSRF-protected GET route first")]
    MissingCsrfCookie { name: String },
    #[error("no WebSocket route matches `{path}`")]
    WebSocketRouteNotFound { path: String },
    #[error("test JSON serialization failed")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
struct TestCsrfConfig {
    cookie_name: String,
    header_name: HeaderName,
    form_field: String,
}

impl Default for TestCsrfConfig {
    fn default() -> Self {
        Self {
            cookie_name: DEFAULT_CSRF_COOKIE.into(),
            header_name: HeaderName::from_static(DEFAULT_CSRF_HEADER),
            form_field: DEFAULT_CSRF_FIELD.into(),
        }
    }
}

#[derive(Clone)]
pub struct TestClient {
    router: Router,
    middlewares: Arc<Vec<MiddlewareFn>>,
    cookies: BTreeMap<String, String>,
    default_headers: HeaderMap,
    csrf: TestCsrfConfig,
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

    pub fn try_text(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.body)
    }

    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }

    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

/// Result of matching a path against the router's WebSocket route table. This
/// verifies route registration, route name, and extracted parameters; it does
/// not create a network socket.
#[must_use]
#[derive(Clone, Debug)]
pub struct TestWebSocket {
    pub path: String,
    pub connected: bool,
    pub route_name: Option<String>,
    pub params: Vec<(String, String)>,
}

impl TestWebSocket {
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }

    pub fn require_match(&self) -> TestClientResult<&Self> {
        if self.connected {
            Ok(self)
        } else {
            Err(TestClientError::WebSocketRouteNotFound {
                path: self.path.clone(),
            })
        }
    }
}

impl TestClient {
    pub fn new(router: Router) -> Self {
        let mut default_headers = HeaderMap::new();
        default_headers.insert(HOST, HeaderValue::from_static(DEFAULT_TEST_HOST));
        default_headers.insert(
            HeaderName::from_static("origin"),
            HeaderValue::from_static(DEFAULT_TEST_ORIGIN),
        );
        Self {
            router,
            middlewares: Arc::new(Vec::new()),
            cookies: BTreeMap::new(),
            default_headers,
            csrf: TestCsrfConfig::default(),
        }
    }

    pub fn with_middleware(mut self, middleware: Vec<MiddlewareFn>) -> Self {
        self.middlewares = Arc::new(middleware);
        self
    }

    /// Headers added to every request. A new client starts with
    /// `Host: app.test` and `Origin: http://app.test`, which models a browser
    /// making same-origin requests and exercises Origin checks by default.
    pub fn default_headers(&self) -> &HeaderMap {
        &self.default_headers
    }

    pub fn default_header(&self, name: &str) -> Option<&str> {
        self.default_headers
            .get(name)
            .and_then(|value| value.to_str().ok())
    }

    pub fn set_default_header(&mut self, name: &str, value: &str) -> TestClientResult<&mut Self> {
        let name = parse_header_name(name)?;
        let value = parse_header_value(name.as_str(), value)?;
        self.default_headers.insert(name, value);
        Ok(self)
    }

    pub fn with_default_header(mut self, name: &str, value: &str) -> TestClientResult<Self> {
        self.set_default_header(name, value)?;
        Ok(self)
    }

    pub fn clear_default_header(&mut self, name: &str) -> TestClientResult<Option<HeaderValue>> {
        let name = parse_header_name(name)?;
        Ok(self.default_headers.remove(name))
    }

    /// Set a complete same-origin URL and keep the `Host` and `Origin`
    /// headers consistent. Ports are preserved.
    pub fn set_same_origin(&mut self, origin: &str) -> TestClientResult<&mut Self> {
        let (origin, host) = parse_same_origin(origin)?;
        self.set_default_header("origin", &origin)?;
        self.set_default_header("host", &host)?;
        Ok(self)
    }

    pub fn with_same_origin(mut self, origin: &str) -> TestClientResult<Self> {
        self.set_same_origin(origin)?;
        Ok(self)
    }

    /// Synchronize helper field/header names with a non-default CSRF policy.
    pub fn set_csrf_config(&mut self, config: &CsrfConfig) -> TestClientResult<&mut Self> {
        let header_name = parse_header_name(&config.header_name)?;
        if !valid_cookie_name(&config.cookie_name) {
            return Err(TestClientError::InvalidCookie {
                name: config.cookie_name.clone(),
            });
        }
        self.csrf = TestCsrfConfig {
            cookie_name: config.cookie_name.clone(),
            header_name,
            form_field: config.form_field.clone(),
        };
        Ok(self)
    }

    pub fn with_csrf_config(mut self, config: &CsrfConfig) -> TestClientResult<Self> {
        self.set_csrf_config(config)?;
        Ok(self)
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    pub fn cookies(&self) -> impl ExactSizeIterator<Item = (&str, &str)> {
        self.cookies
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub fn set_cookie(&mut self, name: &str, value: &str) -> TestClientResult<&mut Self> {
        if !valid_cookie_name(name) || !valid_cookie_value(value) {
            return Err(TestClientError::InvalidCookie { name: name.into() });
        }
        self.cookies.insert(name.into(), value.into());
        Ok(self)
    }

    pub fn remove_cookie(&mut self, name: &str) -> Option<String> {
        self.cookies.remove(name)
    }

    pub fn clear_cookies(&mut self) {
        self.cookies.clear();
    }

    pub fn csrf_token(&self) -> Option<&str> {
        self.cookie(&self.csrf.cookie_name)
    }

    /// Visit a safe route so [`super::csrf::CsrfProtection`] can issue its
    /// readable double-submit cookie, then return the captured token.
    pub async fn fetch_csrf(&mut self, uri: &str) -> TestClientResult<String> {
        self.try_get(uri).await?;
        self.required_csrf_token()
    }

    pub async fn get(&mut self, uri: &str) -> TestResponse {
        convenience(self.try_get(uri).await)
    }

    pub async fn try_get(&mut self, uri: &str) -> TestClientResult<TestResponse> {
        self.try_request(Method::GET, uri, Bytes::new(), None).await
    }

    pub async fn form<I, K, V>(&mut self, method: Method, uri: &str, values: I) -> TestResponse
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        convenience(self.try_form(method, uri, values).await)
    }

    pub async fn try_form<I, K, V>(
        &mut self,
        method: Method,
        uri: &str,
        values: I,
    ) -> TestClientResult<TestResponse>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.try_form_with_headers(method, uri, values, HeaderMap::new())
            .await
    }

    pub async fn try_form_with_headers<I, K, V>(
        &mut self,
        method: Method,
        uri: &str,
        values: I,
        headers: HeaderMap,
    ) -> TestClientResult<TestResponse>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let body = encode_form(values);
        self.try_request_with_headers(
            method,
            uri,
            Bytes::from(body),
            Some("application/x-www-form-urlencoded"),
            headers,
        )
        .await
    }

    /// Submit a classic form with the captured CSRF token in the configured
    /// hidden-field name.
    pub async fn csrf_form<I, K, V>(
        &mut self,
        method: Method,
        uri: &str,
        values: I,
    ) -> TestClientResult<TestResponse>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let token = self.required_csrf_token()?;
        let field = self.csrf.form_field.clone();
        let mut values = values
            .into_iter()
            .map(|(name, value)| (name.as_ref().to_string(), value.as_ref().to_string()))
            .filter(|(name, _)| name != &field)
            .collect::<Vec<_>>();
        values.push((field, token));
        self.try_form(method, uri, values).await
    }

    pub async fn json<T>(&mut self, method: Method, uri: &str, value: &T) -> TestResponse
    where
        T: Serialize + ?Sized,
    {
        convenience(self.try_json(method, uri, value).await)
    }

    pub async fn try_json<T>(
        &mut self,
        method: Method,
        uri: &str,
        value: &T,
    ) -> TestClientResult<TestResponse>
    where
        T: Serialize + ?Sized,
    {
        self.try_json_with_headers(method, uri, value, HeaderMap::new())
            .await
    }

    pub async fn try_json_with_headers<T>(
        &mut self,
        method: Method,
        uri: &str,
        value: &T,
        headers: HeaderMap,
    ) -> TestClientResult<TestResponse>
    where
        T: Serialize + ?Sized,
    {
        let body = serde_json::to_vec(value)?;
        self.try_request_with_headers(
            method,
            uri,
            Bytes::from(body),
            Some("application/json"),
            headers,
        )
        .await
    }

    /// Submit JSON with the captured token in the configured CSRF header.
    pub async fn csrf_json<T>(
        &mut self,
        method: Method,
        uri: &str,
        value: &T,
    ) -> TestClientResult<TestResponse>
    where
        T: Serialize + ?Sized,
    {
        let headers = self.csrf_headers()?;
        self.try_json_with_headers(method, uri, value, headers)
            .await
    }

    pub async fn action<T>(&mut self, token: &str, input: T) -> TestResponse
    where
        T: Serialize,
    {
        convenience(self.try_action(token, input).await)
    }

    pub async fn try_action<T>(&mut self, token: &str, input: T) -> TestClientResult<TestResponse>
    where
        T: Serialize,
    {
        self.try_action_with_headers(token, input, HeaderMap::new())
            .await
    }

    pub async fn try_action_with_headers<T>(
        &mut self,
        token: &str,
        input: T,
        headers: HeaderMap,
    ) -> TestClientResult<TestResponse>
    where
        T: Serialize,
    {
        let input = serde_json::to_value(input)?;
        let body = serde_json::json!({"t": token, "i": input, "ts": now_secs()});
        self.try_json_with_headers(Method::POST, "/api/a", &body, headers)
            .await
    }

    /// Send a plaintext Action envelope with the captured CSRF header. Action
    /// sealing remains an application configuration concern.
    pub async fn csrf_action<T>(&mut self, token: &str, input: T) -> TestClientResult<TestResponse>
    where
        T: Serialize,
    {
        let headers = self.csrf_headers()?;
        self.try_action_with_headers(token, input, headers).await
    }

    pub async fn websocket(&self, path: &str) -> TestWebSocket {
        match self.router.match_ws(path) {
            Some((route, params)) => TestWebSocket {
                path: path.into(),
                connected: true,
                route_name: route.name.clone(),
                params,
            },
            None => TestWebSocket {
                path: path.into(),
                connected: false,
                route_name: None,
                params: Vec::new(),
            },
        }
    }

    pub async fn try_websocket(&self, path: &str) -> TestClientResult<TestWebSocket> {
        let matched = self.websocket(path).await;
        matched.require_match()?;
        Ok(matched)
    }

    /// Backwards-compatible convenience wrapper. Prefer [`Self::try_request`]
    /// when asserting malformed request inputs.
    pub async fn request(
        &mut self,
        method: Method,
        uri: &str,
        body: Bytes,
        content_type: Option<&str>,
    ) -> TestResponse {
        convenience(self.try_request(method, uri, body, content_type).await)
    }

    pub async fn try_request(
        &mut self,
        method: Method,
        uri: &str,
        body: Bytes,
        content_type: Option<&str>,
    ) -> TestClientResult<TestResponse> {
        self.try_request_with_headers(method, uri, body, content_type, HeaderMap::new())
            .await
    }

    pub async fn try_request_with_headers(
        &mut self,
        method: Method,
        uri: &str,
        body: Bytes,
        content_type: Option<&str>,
        request_headers: HeaderMap,
    ) -> TestClientResult<TestResponse> {
        let mut headers = self.default_headers.clone();
        if let Some(content_type) = content_type {
            headers.insert(
                CONTENT_TYPE,
                parse_header_value(CONTENT_TYPE.as_str(), content_type)?,
            );
        }
        headers.extend(request_headers);
        if !self.cookies.is_empty() && !headers.contains_key(COOKIE) {
            let cookie = self
                .cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            headers.insert(COOKIE, parse_header_value(COOKIE.as_str(), &cookie)?);
        }
        let uri: Uri = uri.parse().map_err(|source| TestClientError::InvalidUri {
            uri: uri.into(),
            source,
        })?;
        let request = Request::new(method, uri, headers, body);
        let response = self
            .router
            .dispatch(request, Arc::clone(&self.middlewares))
            .await;
        self.capture_cookies(&response);
        let (status, headers, body) = response.into_status_headers_body().await;
        Ok(TestResponse {
            status,
            headers,
            body,
        })
    }

    fn capture_cookies(&mut self, response: &Response) {
        for value in response.headers().get_all("set-cookie") {
            let Ok(raw) = value.to_str() else {
                continue;
            };
            let Some((name, value)) = raw
                .split(';')
                .next()
                .and_then(|first| first.split_once('='))
            else {
                continue;
            };
            let name = name.trim();
            let value = value.trim();
            if name.is_empty() {
                continue;
            }
            if deletes_cookie(raw) {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.into(), value.into());
            }
        }
    }

    fn required_csrf_token(&self) -> TestClientResult<String> {
        self.csrf_token()
            .map(str::to_string)
            .ok_or_else(|| TestClientError::MissingCsrfCookie {
                name: self.csrf.cookie_name.clone(),
            })
    }

    fn csrf_headers(&self) -> TestClientResult<HeaderMap> {
        let token = self.required_csrf_token()?;
        let mut headers = HeaderMap::new();
        headers.insert(
            self.csrf.header_name.clone(),
            parse_header_value(self.csrf.header_name.as_str(), &token)?,
        );
        Ok(headers)
    }
}

fn convenience<T>(result: TestClientResult<T>) -> T {
    result.unwrap_or_else(|error| panic!("test client request error: {error}"))
}

fn parse_header_name(name: &str) -> TestClientResult<HeaderName> {
    HeaderName::from_bytes(name.as_bytes()).map_err(|source| TestClientError::InvalidHeaderName {
        name: name.into(),
        source,
    })
}

fn parse_header_value(name: &str, value: &str) -> TestClientResult<HeaderValue> {
    HeaderValue::from_str(value).map_err(|source| TestClientError::InvalidHeaderValue {
        name: name.into(),
        source,
    })
}

fn parse_same_origin(origin: &str) -> TestClientResult<(String, String)> {
    let raw = origin.trim().trim_end_matches('/');
    let invalid = || TestClientError::InvalidOrigin {
        origin: origin.into(),
    };
    let uri: Uri = raw.parse().map_err(|_| invalid())?;
    let scheme = uri.scheme_str().ok_or_else(&invalid)?;
    let authority = uri.authority().ok_or_else(&invalid)?.as_str();
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or_default();
    if !matches!(scheme, "http" | "https")
        || authority.is_empty()
        || authority.contains('@')
        || !matches!(path_and_query, "" | "/")
    {
        return Err(invalid());
    }
    Ok((format!("{scheme}://{authority}"), authority.into()))
}

fn encode_form<I, K, V>(values: I) -> String
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    values
        .into_iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                url_encode(key.as_ref()),
                url_encode(value.as_ref())
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn valid_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn valid_cookie_value(value: &str) -> bool {
    value.bytes().all(|byte| {
        matches!(
            byte,
            0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e
        )
    })
}

fn deletes_cookie(raw: &str) -> bool {
    raw.split(';').skip(1).any(|attribute| {
        let Some((name, value)) = attribute.trim().split_once('=') else {
            return false;
        };
        name.trim().eq_ignore_ascii_case("max-age")
            && value
                .trim()
                .parse::<i64>()
                .is_ok_and(|seconds| seconds <= 0)
    })
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
    use crate::core::controller::text;
    use crate::core::csrf::CsrfProtection;
    use crate::core::routing::Route;
    use crate::core::ws::WsSocket;
    use http::StatusCode;

    async fn hello(_: Request) -> Response {
        text("hello")
    }

    async fn browser_headers(req: Request) -> Response {
        text(format!(
            "{}|{}",
            req.header_or("host", "-"),
            req.header_or("origin", "-")
        ))
    }

    async fn set_cookies(_: Request) -> Response {
        text("set")
            .with_cookie("alpha", "one")
            .with_cookie("beta", "two")
    }

    async fn clear_alpha(_: Request) -> Response {
        text("clear").with_clear_cookie("alpha")
    }

    async fn echo_cookie(req: Request) -> Response {
        text(req.header_or("cookie", "-").to_string())
    }

    async fn echo_body(req: Request) -> Response {
        text(req.body_string())
    }

    async fn websocket_handler(_socket: WsSocket) {}

    #[tokio::test]
    async fn visits_route_with_same_origin_browser_headers() {
        let router = Route::get("/", hello)
            .register()
            .merge(Route::get("/headers", browser_headers).register());
        let mut client = TestClient::new(router);

        assert_eq!(client.get("/").await.text(), "hello");
        assert_eq!(
            client.get("/headers").await.text(),
            "app.test|http://app.test"
        );

        client
            .set_same_origin("https://example.test:8443/")
            .unwrap();
        assert_eq!(
            client.get("/headers").await.text(),
            "example.test:8443|https://example.test:8443"
        );
    }

    #[tokio::test]
    async fn cookie_jar_captures_multiple_values_and_removals() {
        let router = Route::get("/cookies/set", set_cookies)
            .register()
            .merge(Route::get("/cookies/clear", clear_alpha).register())
            .merge(Route::get("/cookies/echo", echo_cookie).register());
        let mut client = TestClient::new(router);

        let response = client.get("/cookies/set").await;
        assert_eq!(response.headers.get_all("set-cookie").iter().count(), 2);
        assert_eq!(client.cookie("alpha"), Some("one"));
        assert_eq!(client.cookie("beta"), Some("two"));
        assert_eq!(client.cookies().len(), 2);
        assert_eq!(
            client.get("/cookies/echo").await.text(),
            "alpha=one; beta=two"
        );

        client.get("/cookies/clear").await;
        assert_eq!(client.cookie("alpha"), None);
        assert_eq!(client.cookie("beta"), Some("two"));
        assert_eq!(client.get("/cookies/echo").await.text(), "beta=two");

        client.clear_cookies();
        assert_eq!(client.cookies().len(), 0);
    }

    #[tokio::test]
    async fn csrf_helpers_cover_classic_forms_json_and_action_envelopes() {
        let router = Route::get("/csrf", hello)
            .register()
            .merge(Route::post("/form", echo_body).register())
            .merge(Route::post("/json", echo_body).register())
            .merge(Route::post("/api/a", echo_body).register());
        let csrf = CsrfProtection::new(CsrfConfig::default()).middleware();
        let mut client = TestClient::new(router).with_middleware(vec![csrf]);

        let missing = client
            .csrf_form(Method::POST, "/form", [("name", "Ada Lovelace")])
            .await
            .unwrap_err();
        assert!(matches!(missing, TestClientError::MissingCsrfCookie { .. }));

        let rejected = client
            .form(Method::POST, "/form", [("name", "Ada Lovelace")])
            .await;
        assert_eq!(rejected.status, StatusCode::FORBIDDEN);
        assert_eq!(client.csrf_token(), None);

        let token = client.fetch_csrf("/csrf").await.unwrap();
        assert_eq!(client.csrf_token(), Some(token.as_str()));

        let form = client
            .csrf_form(Method::POST, "/form", [("name", "Ada Lovelace")])
            .await
            .unwrap();
        assert_eq!(form.status, StatusCode::OK);
        assert!(form.text().contains("name=Ada+Lovelace"));
        assert!(form.text().contains("_csrf="));

        let raw_json = client
            .json(Method::POST, "/json", &serde_json::json!({"ok": true}))
            .await;
        assert_eq!(raw_json.status, StatusCode::FORBIDDEN);

        let json = client
            .csrf_json(Method::POST, "/json", &serde_json::json!({"ok": true}))
            .await
            .unwrap();
        assert_eq!(json.status, StatusCode::OK);
        assert_eq!(json.json::<serde_json::Value>().unwrap()["ok"], true);

        let action = client
            .csrf_action("deadbeef", serde_json::json!({"user_id": 7}))
            .await
            .unwrap();
        assert_eq!(action.status, StatusCode::OK);
        let envelope = action.json::<serde_json::Value>().unwrap();
        assert_eq!(envelope["t"], "deadbeef");
        assert_eq!(envelope["i"]["user_id"], 7);
        assert!(envelope["ts"].as_u64().is_some());

        client
            .set_default_header("origin", "https://attacker.test")
            .unwrap();
        let rejected_origin = client
            .csrf_json(Method::POST, "/json", &serde_json::json!({"ok": true}))
            .await
            .unwrap();
        assert_eq!(rejected_origin.status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn malformed_request_configuration_returns_typed_errors() {
        let mut client = TestClient::new(Route::get("/", hello).register());

        assert!(matches!(
            client.set_same_origin("file:///tmp/test"),
            Err(TestClientError::InvalidOrigin { .. })
        ));
        assert!(matches!(
            client.set_default_header("bad header", "value"),
            Err(TestClientError::InvalidHeaderName { .. })
        ));
        assert!(matches!(
            client.set_default_header("x-test", "line\nbreak"),
            Err(TestClientError::InvalidHeaderValue { .. })
        ));
        assert!(matches!(
            client.try_get("/bad\nuri").await,
            Err(TestClientError::InvalidUri { .. })
        ));
    }

    #[tokio::test]
    async fn websocket_match_reports_name_params_and_missing_route() {
        let router = Route::ws("/ws/:room/*tail", websocket_handler)
            .name("chat.stream")
            .register();
        let client = TestClient::new(router);

        let matched = client
            .try_websocket("/ws/general/messages/today")
            .await
            .unwrap();
        assert!(matched.connected);
        assert_eq!(matched.route_name.as_deref(), Some("chat.stream"));
        assert_eq!(matched.param("room"), Some("general"));
        assert_eq!(matched.param("tail"), Some("messages/today"));

        let missing = client.websocket("/missing").await;
        assert!(!missing.connected);
        assert!(matches!(
            missing.require_match(),
            Err(TestClientError::WebSocketRouteNotFound { .. })
        ));
    }
}
