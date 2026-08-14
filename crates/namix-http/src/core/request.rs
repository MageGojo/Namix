use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::routing::{IntoRouteName, NamedRoute, RouteCatalog};
use super::upload::{MultipartBag, UploadedFile, is_multipart, parse_multipart};
use super::validate::{Field, Rule, Validated, ValidationError, Validator};

/// Peer address recorded by the HTTP server before proxy processing.
///
/// Rate-limit middleware uses this value by default.  A deployment that trusts
/// a reverse proxy may replace it after validating its proxy boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

/// Typed JSON body decoding failure. It converts to a client-facing
/// [`super::error::AppError::BadRequest`] when a controller uses `?`.
#[derive(Debug, Error)]
#[error("invalid JSON request body")]
pub struct JsonBodyError {
    #[source]
    source: serde_json::Error,
}

impl From<JsonBodyError> for super::error::AppError {
    fn from(error: JsonBodyError) -> Self {
        tracing::debug!(error = ?error.source, "JSON request body decode failed");
        Self::bad_request("invalid JSON request body")
    }
}

/// 一次入站请求（框架层）。
///
/// 读写尽量用短方法，避免直接抠 `HeaderMap` / `HeaderValue`：
///
/// ```ignore
/// let token = req.header("x-admin-token");
/// let page = req.query_or("page", "1");
/// req.set_attr("admin", "1");
/// req.set::<AuthUser>(user);
/// req.redirect_to(UserRoute::Login);
/// ```
#[derive(Clone)]
pub struct Request {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    /// 按路由模式中 `:param` 出现顺序排列。
    params: Vec<(String, String)>,
    /// 字符串上下文（中间件互相传轻量数据）。
    attrs: HashMap<String, String>,
    /// 类型化上下文（中间件/控制器传结构体）。
    store: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// 命名路由表（启动时注入）。
    routes: Option<Arc<RouteCatalog>>,
    /// 解析过的 multipart（惰性，克隆共享）。
    form_bag: Arc<OnceLock<MultipartBag>>,
    /// query + body 合并后的输入袋（Laravel `$request->input`，惰性）。
    inputs: Arc<OnceLock<HashMap<String, String>>>,
}

impl Request {
    pub(crate) fn new(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Self {
        Self {
            method,
            uri,
            headers,
            body,
            params: Vec::new(),
            attrs: HashMap::new(),
            store: HashMap::new(),
            routes: None,
            form_bag: Arc::new(OnceLock::new()),
            inputs: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn set_params(&mut self, params: Vec<(String, String)>) {
        self.params = params;
    }

    pub(crate) fn set_routes(&mut self, routes: Arc<RouteCatalog>) {
        self.routes = Some(routes);
    }

    // ── 基础字段 ──────────────────────────────────────────────

    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn set_method(&mut self, method: Method) {
        self.method = method;
    }

    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    pub fn path(&self) -> &str {
        self.uri.path()
    }

    pub fn query_string(&self) -> Option<&str> {
        self.uri.query()
    }

    // ── Header ────────────────────────────────────────────────

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    /// `req.header("x-admin-token")` → `Some("namix-dev")`
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn header_or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.header(name).unwrap_or(default)
    }

    pub fn has_header(&self, name: &str) -> bool {
        self.headers.contains_key(name)
    }

    pub fn set_header(&mut self, name: &str, value: impl AsRef<str>) -> &mut Self {
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name),
            HeaderValue::from_str(value.as_ref()),
        ) {
            self.headers.insert(name, value);
        }
        self
    }

    pub fn remove_header(&mut self, name: &str) -> &mut Self {
        self.headers.remove(name);
        self
    }

    /// `Authorization: Bearer xxx` → `Some("xxx")`
    pub fn bearer(&self) -> Option<&str> {
        self.header("authorization")
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
            })
            .map(str::trim)
            .filter(|v| !v.is_empty())
    }

    /// `Cookie: a=1; namix_user=alice` → `req.cookie("namix_user")`
    /// CSRF token for SSR `ViewData` / 自绘 hidden input。中间件未跑时为空串。
    pub fn csrf_token(&self) -> &str {
        super::csrf::token(self).unwrap_or("")
    }

    pub fn cookie(&self, name: &str) -> Option<&str> {
        let raw = self.header("cookie")?;
        for part in raw.split(';') {
            let part = part.trim();
            let Some((k, v)) = part.split_once('=') else {
                continue;
            };
            if k.trim() == name {
                let v = v.trim();
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
        None
    }

    // ── Query / Path 参数 ─────────────────────────────────────

    /// `?page=2` → `req.query("page") == Some("2")`
    pub fn query(&self, name: &str) -> Option<String> {
        let qs = self.uri.query()?;
        for pair in qs.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = match pair.split_once('=') {
                Some((k, v)) => (k, v),
                None => (pair, ""),
            };
            // Query producers such as `URLSearchParams` percent-encode keys
            // (`filter[status]` becomes `filter%5Bstatus%5D`). Decode both
            // sides consistently instead of making framework filters depend
            // on a non-standard raw-bracket URL.
            if query_decode(k) == name {
                return Some(query_decode(v));
            }
        }
        None
    }

    pub fn query_or(&self, name: &str, default: &str) -> String {
        self.query(name).unwrap_or_else(|| default.to_string())
    }

    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn param_or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.param(name).unwrap_or(default)
    }

    pub fn params(&self) -> &[(String, String)] {
        &self.params
    }

    // ── Body ──────────────────────────────────────────────────

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn body_str(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap_or("")
    }

    pub fn body_string(&self) -> String {
        self.body_str().to_string()
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, JsonBodyError> {
        serde_json::from_slice(&self.body).map_err(|source| JsonBodyError { source })
    }

    pub fn set_body(&mut self, body: impl Into<Bytes>) -> &mut Self {
        self.body = body.into();
        self.form_bag = Arc::new(OnceLock::new());
        self.inputs = Arc::new(OnceLock::new());
        self
    }

    fn input_map(&self) -> &HashMap<String, String> {
        self.inputs
            .get_or_init(|| crate::core::server_fn::expand_input_map(self).unwrap_or_default())
    }

    /// query + JSON / urlencoded / multipart 字段。对齐 Laravel `$request->all()`。
    pub fn inputs(&self) -> &HashMap<String, String> {
        self.input_map()
    }

    /// 对齐 Laravel `$request->input('title')`：query 与 body 合并，同名以 body 为准。
    pub fn input(&self, name: &str) -> Option<&str> {
        self.input_map().get(name).map(String::as_str)
    }

    pub fn input_or<'a>(&'a self, name: &str, default: &'a str) -> &'a str {
        self.input(name).unwrap_or(default)
    }

    /// 对齐 Laravel `$request->ip()`。
    pub fn ip(&self) -> Option<IpAddr> {
        self.client_ip()
    }

    /// Parsed `multipart/form-data` fields and files (empty for other bodies).
    pub fn form_bag(&self) -> &MultipartBag {
        self.form_bag.get_or_init(|| {
            let content_type = self.header_or("content-type", "");
            if is_multipart(content_type) {
                parse_multipart(&self.body, content_type).unwrap_or_default()
            } else {
                MultipartBag::default()
            }
        })
    }

    pub fn file(&self, name: &str) -> Option<&UploadedFile> {
        self.form_bag().files.get(name)
    }

    pub fn files(&self) -> &HashMap<String, UploadedFile> {
        &self.form_bag().files
    }

    pub fn form_input(&self, name: &str) -> Option<&str> {
        self.form_bag().fields.get(name).map(String::as_str)
    }

    // ── Attr（字符串上下文）────────────────────────────────────

    /// 中间件之间传递轻量字符串：`req.set_attr("role", "admin")`
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }

    pub fn attr_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.attr(key).unwrap_or(default)
    }

    pub fn set_attr(&mut self, key: impl Into<String>, value: impl Into<String>) -> &mut Self {
        self.attrs.insert(key.into(), value.into());
        self
    }

    pub fn remove_attr(&mut self, key: &str) -> &mut Self {
        self.attrs.remove(key);
        self
    }

    pub fn attrs(&self) -> &HashMap<String, String> {
        &self.attrs
    }

    // ── 类型化上下文 ──────────────────────────────────────────

    /// `req.set(AdminUser { id: 1 })`
    pub fn set<T: Send + Sync + 'static>(&mut self, value: T) -> &mut Self {
        self.store.insert(TypeId::of::<T>(), Arc::new(value));
        self
    }

    /// `req.get::<AdminUser>()`
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.store
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }

    pub fn take_cloned<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        self.get::<T>().cloned()
    }

    /// Socket peer IP as installed by the Namix server. 也可用 [`Self::ip`]。
    pub fn client_ip(&self) -> Option<IpAddr> {
        self.get::<ClientIp>().map(|ClientIp(ip)| *ip)
    }

    /// Set a validated client IP (primarily for a trusted proxy middleware or
    /// deterministic request tests).
    pub fn set_client_ip(&mut self, ip: IpAddr) -> &mut Self {
        self.set(ClientIp(ip))
    }

    // ── Redirect（中间件里直接跳，不进控制器）────────────────

    /// `return req.redirect("/login");`
    pub fn redirect(&self, to: impl AsRef<str>) -> crate::core::response::Response {
        crate::core::response::Response::redirect(to)
    }

    pub fn redirect_permanent(&self, to: impl AsRef<str>) -> crate::core::response::Response {
        crate::core::response::Response::redirect_permanent(to)
    }

    /// 跳到登录页，并带上当前地址：`/login?redirect=/profile/1`
    pub fn redirect_guest(&self, login: impl AsRef<str>) -> crate::core::response::Response {
        let login = login.as_ref();
        let here = match self.query_string() {
            Some(q) => format!("{}?{q}", self.path()),
            None => self.path().to_string(),
        };
        let sep = if login.contains('?') { '&' } else { '?' };
        let to = format!("{login}{sep}redirect={}", query_encode(&here));
        crate::core::response::Response::redirect(to)
    }

    /// 上一页：`?redirect=` → Referer 路径 → `None`
    pub fn previous_url(&self) -> Option<String> {
        if let Some(r) = self.query("redirect")
            && is_local_path(&r)
        {
            return Some(r);
        }
        if let Some(referer) = self.header("referer") {
            let path = path_from_referer(referer);
            if is_local_path(&path) {
                return Some(path);
            }
        }
        None
    }

    /// 退回上一页；没有上一页时回 `/`。
    pub fn redirect_back(&self) -> crate::core::response::Response {
        self.redirect_back_or("/")
    }

    pub fn redirect_back_or(&self, fallback: impl AsRef<str>) -> crate::core::response::Response {
        match self.previous_url() {
            Some(url) => self.redirect(url),
            None => self.redirect(fallback.as_ref()),
        }
    }

    /// 退回上一页；没有则按路由名（字符串或 `route::user::home`）。
    pub fn redirect_back_or_route(
        &self,
        name: impl IntoRouteName,
    ) -> crate::core::response::Response {
        match self.previous_url() {
            Some(url) => self.redirect(url),
            None => self.redirect_route(name),
        }
    }

    // ── 命名路由跳转（优先 `route::user::login`，也兼容字符串）──

    pub fn url(&self, name: impl IntoRouteName) -> Option<String> {
        self.url_with(name, &[])
    }

    pub fn url_with(&self, name: impl IntoRouteName, params: &[(&str, &str)]) -> Option<String> {
        let name = name.into_route_name();
        self.routes.as_ref()?.url(&name, params)
    }

    pub fn redirect_route(&self, name: impl IntoRouteName) -> crate::core::response::Response {
        let name = name.into_route_name();
        match self.url(name.as_str()) {
            Some(url) => self.redirect(url),
            None => crate::core::response::Response::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::core::content_type::ContentType::Text,
                format!("unknown route name: {name}"),
            ),
        }
    }

    pub fn redirect_route_with(
        &self,
        name: impl IntoRouteName,
        params: &[(&str, &str)],
    ) -> crate::core::response::Response {
        let name = name.into_route_name();
        match self.url_with(name.as_str(), params) {
            Some(url) => self.redirect(url),
            None => crate::core::response::Response::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                crate::core::content_type::ContentType::Text,
                format!("unknown or incomplete route: {name}"),
            ),
        }
    }

    pub fn redirect_guest_route(
        &self,
        name: impl IntoRouteName,
    ) -> crate::core::response::Response {
        match self.url(name) {
            Some(url) => self.redirect_guest(url),
            None => self.redirect_guest("/login"),
        }
    }

    pub fn url_to<R: NamedRoute>(&self, route: R) -> Option<String> {
        self.url(route)
    }

    pub fn url_to_with<R: NamedRoute>(&self, route: R, params: &[(&str, &str)]) -> Option<String> {
        self.url_with(route, params)
    }

    /// 当前应用的命名路由表（供前端 / 调试）。
    pub fn route_catalog(&self) -> Option<&RouteCatalog> {
        self.routes.as_deref()
    }

    /// Ziggy 风格 JSON：`{ "user.profile": { "uri": "/profile/:id", "methods": ["GET"] } }`
    pub fn routes_json(&self) -> Option<String> {
        self.routes.as_ref()?.to_json().ok()
    }

    /// Laravel：`view('login')->with(...)->…` —— 见 [`crate::features::pages::Controller::view`]。
    #[cfg(feature = "pages")]
    pub fn view(&self, component: impl Into<String>) -> crate::features::pages::ViewBag<'_> {
        crate::features::pages::Controller::view(self, component)
    }

    /// 类型化：`req.render(Login { .. })`。
    #[cfg(feature = "pages")]
    pub fn render<P: crate::features::pages::ViewPage>(
        &self,
        page: P,
    ) -> crate::core::response::Response {
        crate::features::pages::Controller::render(self, page)
    }

    /// 兼容旧名 → [`Request::render`]。
    #[cfg(feature = "pages")]
    pub fn page<P: crate::features::pages::ViewPage>(
        &self,
        props: P,
    ) -> crate::core::response::Response {
        self.render(props)
    }

    pub fn redirect_to<R: NamedRoute>(&self, route: R) -> crate::core::response::Response {
        self.redirect_route(route)
    }

    pub fn redirect_guest_to<R: NamedRoute>(&self, route: R) -> crate::core::response::Response {
        self.redirect_guest_route(route)
    }

    pub fn redirect_back_or_to<R: NamedRoute>(&self, route: R) -> crate::core::response::Response {
        self.redirect_back_or_route(route)
    }

    // ── 验证器 ────────────────────────────────────────────────

    pub fn validator(&self) -> Validator<'_> {
        Validator::from_request(self)
    }

    pub fn validate<F: Field>(&self, rules: &[(F, &[Rule])]) -> Result<Validated, ValidationError> {
        let mut v = self.validator();
        for (field, rs) in rules {
            v = v.rules(*field, rs);
        }
        v.validate()
    }
}

/// 仅接受当前站点内的绝对路径，适合登录后跳转和 `redirect` 参数。
///
/// 拒绝完整 URL、协议相对 URL（`//host`）、反斜杠变体和控制字符。
pub fn is_local_path(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && !value.starts_with("/\\")
        && !value.chars().any(char::is_control)
}

pub(crate) fn query_encode_pub(s: &str) -> String {
    query_encode(s)
}

pub(crate) fn query_decode_pub(s: &str) -> String {
    query_decode(s)
}

fn path_from_referer(referer: &str) -> String {
    if let Ok(uri) = referer.parse::<Uri>() {
        let path = uri.path();
        match uri.query() {
            Some(q) => format!("{path}?{q}"),
            None => path.to_string(),
        }
    } else if referer.starts_with('/') {
        referer.to_string()
    } else {
        "/".into()
    }
}

fn query_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn query_decode(s: &str) -> String {
    // 轻量 percent-decode（足够覆盖常见 query）
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let h =
                    u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16);
                if let Ok(b) = h {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(uri: &str) -> Request {
        Request::new(
            Method::GET,
            uri.parse().expect("valid test uri"),
            HeaderMap::new(),
            Bytes::new(),
        )
    }

    #[test]
    fn local_paths_reject_external_redirect_variants() {
        assert!(is_local_path("/dashboard?tab=profile"));
        assert!(is_local_path("/"));
        assert!(!is_local_path("https://example.test/path"));
        assert!(!is_local_path("//example.test/path"));
        assert!(!is_local_path("/\\example.test/path"));
        assert!(!is_local_path("/path\r\nlocation: /other"));
    }

    #[test]
    fn previous_url_only_uses_local_redirect_query() {
        let local = request("/login?redirect=%2Fdashboard%3Ftab%3Dprofile");
        assert_eq!(
            local.previous_url().as_deref(),
            Some("/dashboard?tab=profile")
        );

        let external = request("/login?redirect=https%3A%2F%2Fexample.test");
        assert_eq!(external.previous_url(), None);
    }

    #[test]
    fn query_decodes_percent_encoded_keys() {
        let request = request("/posts?filter%5Bstatus%5D=published");
        assert_eq!(
            request.query("filter[status]").as_deref(),
            Some("published")
        );
    }

    #[test]
    fn previous_url_rejects_scheme_relative_referer() {
        let mut request = request("/login");
        request.set_header("referer", "//attacker.test/path");
        assert_eq!(request.previous_url(), None);
    }

    #[test]
    fn json_decode_errors_convert_to_bad_request() {
        let request = Request::new(
            Method::POST,
            Uri::from_static("/api/items"),
            HeaderMap::new(),
            Bytes::from_static(b"{not-json}"),
        );
        let error = request.json::<serde_json::Value>().unwrap_err();
        let app_error: super::super::error::AppError = error.into();
        assert_eq!(app_error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn input_merges_query_and_json_body() {
        let mut request = Request::new(
            Method::POST,
            "/save?from=query".parse().expect("uri"),
            HeaderMap::new(),
            Bytes::from_static(br#"{"title":"hello"}"#),
        );
        request.set_header("content-type", "application/json");
        assert_eq!(request.input("from"), Some("query"));
        assert_eq!(request.input("title"), Some("hello"));
        assert_eq!(request.input_or("missing", "x"), "x");
    }

    #[test]
    fn input_reads_urlencoded_body() {
        let mut request = Request::new(
            Method::POST,
            Uri::from_static("/save"),
            HeaderMap::new(),
            Bytes::from_static(b"title=namix&ok=1"),
        );
        request.set_header("content-type", "application/x-www-form-urlencoded");
        assert_eq!(request.input("title"), Some("namix"));
        assert_eq!(request.inputs().get("ok").map(String::as_str), Some("1"));
    }
}
