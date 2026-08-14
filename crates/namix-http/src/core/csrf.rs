//! Browser CSRF / Origin protection.
//!
//! The middleware uses a double-submit token and requires a same-origin
//! `Origin` header for browser state-changing requests.  Bearer-only API
//! requests stay stateless and are therefore not subject to the cookie check.

use rand::RngCore;

use super::content_type::ContentType;
use super::middleware::{MiddlewareFn, Next, wrap_middleware};
use super::request::Request;
use super::response::{CookieOptions, Response};
use http::{Method, StatusCode};

/// Request-scoped CSRF token.  It is populated by [`CsrfProtection`], so a
/// classic server-rendered form can render [`hidden_field`].
#[derive(Clone, Debug)]
pub struct CsrfToken(String);

impl CsrfToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Browser CSRF policy.
#[derive(Clone, Debug)]
pub struct CsrfConfig {
    pub enabled: bool,
    pub cookie_name: String,
    pub header_name: String,
    pub form_field: String,
    /// Complete origins such as `https://app.example.test`.  When empty, the
    /// request `Host` header is used and both HTTP and HTTPS are accepted.
    pub trusted_origins: Vec<String>,
    pub secure_cookie: bool,
    /// Path prefixes whose signed (`expires` + `signature`) mutations skip CSRF.
    /// Unsigned PUTs on the same paths stay protected.
    pub except_prefixes: Vec<String>,
}

impl Default for CsrfConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cookie_name: "namix_csrf".into(),
            header_name: "x-csrf-token".into(),
            form_field: "_csrf".into(),
            trusted_origins: Vec::new(),
            secure_cookie: false,
            except_prefixes: Vec::new(),
        }
    }
}

/// Stateful-free CSRF middleware.  The token lives in a readable SameSite
/// cookie and must be echoed through a form field or request header.
#[derive(Clone, Debug)]
pub struct CsrfProtection {
    config: CsrfConfig,
}

impl CsrfProtection {
    pub fn new(config: CsrfConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &CsrfConfig {
        &self.config
    }

    /// Return a regular Namix middleware function so it can be installed on
    /// `Boot`, `Router`, or one specific `Route`.
    pub fn middleware(self) -> MiddlewareFn {
        wrap_middleware(move |req, next| {
            let protection = self.clone();
            async move { protection.handle(req, next).await }
        })
    }

    async fn handle(&self, mut req: Request, next: Next) -> Response {
        if !self.config.enabled {
            return next.run(req).await;
        }

        let cookie_token = req.cookie(&self.config.cookie_name).map(str::to_string);
        let token = cookie_token.clone().unwrap_or_else(new_token);
        req.set(CsrfToken(token.clone()));
        req.set(self.config.clone());

        if websocket_requires_same_origin(&req) && !origin_allowed(&req, &self.config) {
            return rejected(&req);
        }

        if requires_protection(&req, &self.config)
            && (!origin_allowed(&req, &self.config) || !token_matches(&req, &self.config, &token))
        {
            return rejected(&req);
        }

        let mut response = next.run(req).await;
        if cookie_token.is_none() {
            response.set_cookie_with_options(
                &self.config.cookie_name,
                &token,
                CookieOptions::csrf(self.config.secure_cookie),
            );
        }
        response
    }
}

/// Render the hidden input required by a classic HTML form.
pub fn hidden_field(req: &Request) -> String {
    let field = req
        .get::<CsrfConfig>()
        .map(|config| config.form_field.as_str())
        .unwrap_or("_csrf");
    let value = token(req).unwrap_or_default();
    format!(
        r#"<input type="hidden" name="{}" value="{}">"#,
        html_attr(field),
        html_attr(value)
    )
}

/// Return the current token for templates which render their own input.
pub fn token(req: &Request) -> Option<&str> {
    req.get::<CsrfToken>().map(CsrfToken::as_str)
}

/// Browser WebSocket handshakes are GET requests and therefore do not use the
/// mutation token check, but cookies are still sent automatically cross-site.
/// Require a same-origin `Origin` for browser-shaped handshakes. A cookie-free
/// bearer client remains compatible even when it supplies no Origin.
fn websocket_requires_same_origin(req: &Request) -> bool {
    if req.method() != Method::GET || !crate::core::ws::is_upgrade_request(req.headers()) {
        return false;
    }
    let bearer_only = req.bearer().is_some() && req.header("cookie").is_none();
    !bearer_only && (req.header("origin").is_some() || req.header("cookie").is_some())
}

fn requires_protection(req: &Request, config: &CsrfConfig) -> bool {
    if matches!(
        req.method(),
        &Method::GET | &Method::HEAD | &Method::OPTIONS | &Method::TRACE
    ) {
        return false;
    }

    if signed_prefix_exempt(req, config) {
        return false;
    }

    // Token-authenticated, cookie-free API clients do not share browser
    // cookies and must remain usable without first visiting a HTML page.
    let bearer_only = req.bearer().is_some() && req.header("cookie").is_none();
    if bearer_only {
        return false;
    }

    // Browser navigations/fetches supply Origin or Fetch Metadata.  A request
    // with a session/CSRF cookie is also treated as browser state.
    req.header("origin").is_some()
        || req.header("sec-fetch-site").is_some()
        || req.cookie(&config.cookie_name).is_some()
        || req.cookie("namix_session").is_some()
}

fn signed_prefix_exempt(req: &Request, config: &CsrfConfig) -> bool {
    if config.except_prefixes.is_empty() {
        return false;
    }
    let path = req.path();
    let matched = config.except_prefixes.iter().any(|prefix| {
        let prefix = prefix.trim_end_matches('/');
        if prefix.is_empty() {
            return false;
        }
        path == prefix || path.starts_with(&format!("{prefix}/"))
    });
    matched
        && req.query("expires").is_some_and(|value| !value.is_empty())
        && req
            .query("signature")
            .is_some_and(|value| !value.is_empty())
}

fn origin_allowed(req: &Request, config: &CsrfConfig) -> bool {
    let Some(origin) = req.header("origin") else {
        return false;
    };
    let origin = normalize_origin(origin);
    let Some(origin) = origin else {
        return false;
    };

    if !config.trusted_origins.is_empty() {
        return config
            .trusted_origins
            .iter()
            .filter_map(|candidate| normalize_origin(candidate))
            .any(|candidate| candidate == origin);
    }

    let Some(host) = req.header("host") else {
        return false;
    };
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .is_some_and(|authority| authority.eq_ignore_ascii_case(host))
}

fn normalize_origin(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/');
    let (scheme, authority) = value.split_once("://")?;
    if !matches!(scheme, "http" | "https")
        || authority.is_empty()
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
        || authority.contains('@')
    {
        return None;
    }
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase()
    ))
}

fn token_matches(req: &Request, config: &CsrfConfig, expected: &str) -> bool {
    let supplied = req
        .header(&config.header_name)
        .map(str::to_string)
        .or_else(|| form_value(req.body_str(), &config.form_field))
        .or_else(|| {
            req.form_input(&config.form_field)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });
    supplied.is_some_and(|value| constant_time_eq(expected.as_bytes(), value.as_bytes()))
}

fn form_value(raw: &str, field: &str) -> Option<String> {
    raw.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (super::request::query_decode_pub(key) == field)
            .then(|| super::request::query_decode_pub(value))
    })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let len = left.len().max(right.len());
    for i in 0..len {
        diff |= usize::from(*left.get(i).unwrap_or(&0) ^ *right.get(i).unwrap_or(&0));
    }
    diff == 0
}

fn new_token() -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut random = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut random);
    random
        .iter()
        .map(|byte| ALPHABET[usize::from(byte & 0x3f)] as char)
        .collect()
}

fn html_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

fn rejected(req: &Request) -> Response {
    let body = serde_json::json!({
        "error": "csrf validation failed",
        "message": "csrf validation failed",
        "errors": { "_": "csrf validation failed" },
    })
    .to_string();
    if req.path().starts_with("/api/")
        || req
            .header("accept")
            .is_some_and(|accept| accept.contains("application/json"))
    {
        Response::new(StatusCode::FORBIDDEN, ContentType::Json, body)
    } else {
        Response::new(
            StatusCode::FORBIDDEN,
            ContentType::Html,
            "<!doctype html><title>Forbidden</title><main><h1>Forbidden</h1><p>csrf validation failed</p></main>",
        )
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::{HeaderMap, HeaderName, HeaderValue, Method, Uri};

    use super::*;

    fn req(method: Method, headers: &[(&str, &str)], body: &str) -> Request {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        Request::new(
            method,
            Uri::from_static("/profile"),
            map,
            Bytes::from(body.to_owned()),
        )
    }

    async fn ok(_req: Request) -> Response {
        Response::new(StatusCode::OK, ContentType::Text, "ok")
    }

    fn terminal() -> Next {
        Next::new(
            std::sync::Arc::new(vec![]),
            0,
            std::sync::Arc::new(|r| Box::pin(ok(r))),
        )
    }

    fn websocket_req(extra_headers: &[(&str, &str)]) -> Request {
        let mut headers = vec![
            ("host", "app.test"),
            ("connection", "Upgrade"),
            ("upgrade", "websocket"),
            ("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="),
        ];
        headers.extend_from_slice(extra_headers);
        req(Method::GET, &headers, "")
    }

    #[tokio::test]
    async fn safe_request_issues_readable_csrf_cookie() {
        let guard = CsrfProtection::new(CsrfConfig::default());
        let response = guard
            .handle(
                req(Method::GET, &[("host", "app.test")], ""),
                Next::new(
                    std::sync::Arc::new(vec![]),
                    0,
                    std::sync::Arc::new(|r| Box::pin(ok(r))),
                ),
            )
            .await;
        let cookie = response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("namix_csrf="));
        assert!(!cookie.contains("HttpOnly"));
    }

    #[tokio::test]
    async fn mutation_requires_same_origin_and_matching_token() {
        let guard = CsrfProtection::new(CsrfConfig::default());
        let response = guard
            .handle(
                req(
                    Method::POST,
                    &[
                        ("host", "app.test"),
                        ("origin", "https://attacker.test"),
                        ("cookie", "namix_csrf=token"),
                    ],
                    "_csrf=token",
                ),
                Next::new(
                    std::sync::Arc::new(vec![]),
                    0,
                    std::sync::Arc::new(|r| Box::pin(ok(r))),
                ),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = guard
            .handle(
                req(
                    Method::POST,
                    &[
                        ("host", "app.test"),
                        ("origin", "https://app.test"),
                        ("cookie", "namix_csrf=token"),
                    ],
                    "_csrf=token",
                ),
                Next::new(
                    std::sync::Arc::new(vec![]),
                    0,
                    std::sync::Arc::new(|r| Box::pin(ok(r))),
                ),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn cookie_websocket_requires_same_origin() {
        let guard = CsrfProtection::new(CsrfConfig::default());

        let response = guard
            .handle(
                websocket_req(&[
                    ("origin", "https://app.test"),
                    ("cookie", "namix_session=session"),
                ]),
                terminal(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);

        let response = guard
            .handle(
                websocket_req(&[
                    ("origin", "https://attacker.test"),
                    ("cookie", "namix_session=session"),
                ]),
                terminal(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let response = guard
            .handle(
                websocket_req(&[("cookie", "namix_session=session")]),
                terminal(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn cookie_free_bearer_websocket_remains_origin_independent() {
        let guard = CsrfProtection::new(CsrfConfig::default());
        let response = guard
            .handle(
                websocket_req(&[("authorization", "Bearer TOKEN")]),
                terminal(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn hidden_field_uses_configured_form_field_name() {
        let mut request = req(Method::GET, &[("host", "app.test")], "");
        request.set(CsrfToken("abc".into()));
        request.set(CsrfConfig {
            form_field: "csrf_token".into(),
            ..CsrfConfig::default()
        });
        let html = hidden_field(&request);
        assert!(html.contains(r#"name="csrf_token""#));
        assert!(html.contains(r#"value="abc""#));
        assert!(!html.contains(r#"name="_csrf""#));
    }

    #[test]
    fn request_csrf_token_reads_installed_token() {
        let mut request = req(Method::GET, &[("host", "app.test")], "");
        assert_eq!(request.csrf_token(), "");
        request.set(CsrfToken("tok-1".into()));
        assert_eq!(request.csrf_token(), "tok-1");
    }

    #[tokio::test]
    async fn signed_storage_put_skips_csrf_on_except_prefixes() {
        let config = CsrfConfig {
            except_prefixes: vec!["/storage".into()],
            ..CsrfConfig::default()
        };
        let guard = CsrfProtection::new(config);
        let request = Request::new(
            Method::PUT,
            Uri::from_static("/storage/a.png?expires=1&signature=abc"),
            {
                let mut map = HeaderMap::new();
                map.insert("host", HeaderValue::from_static("app.test"));
                map.insert("origin", HeaderValue::from_static("https://evil.test"));
                map.insert("cookie", HeaderValue::from_static("namix_csrf=tok"));
                map
            },
            Bytes::from_static(b"file"),
        );
        let response = guard.handle(request, terminal()).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}
