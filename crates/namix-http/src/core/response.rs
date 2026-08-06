use std::path::{Component, Path};

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Response as HttpResponse, StatusCode, header};
use http_body_util::{BodyExt, Full};

use super::content_type::ContentType;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
/// 出站 body：普通响应用 `Full`，SSE 等用流式 `StreamBody`。
pub type Body = http_body_util::combinators::BoxBody<Bytes, BoxError>;

/// Cookie 的常用安全属性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookieOptions {
    pub path: String,
    pub http_only: bool,
    pub secure: bool,
    pub same_site: &'static str,
    pub max_age: Option<u64>,
}

impl Default for CookieOptions {
    fn default() -> Self {
        Self {
            path: "/".into(),
            http_only: true,
            secure: false,
            same_site: "Lax",
            max_age: None,
        }
    }
}

impl CookieOptions {
    /// CSRF double-submit cookie：浏览器脚本需要读取它并放入请求头。
    pub fn csrf(secure: bool) -> Self {
        Self {
            http_only: false,
            secure,
            same_site: "Strict",
            ..Self::default()
        }
    }
}

pub(crate) fn body_full(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into())
        .map_err(|never| match never {})
        .boxed()
}

/// 一次出站响应（框架层）。
///
/// ```ignore
/// let mut resp = next.run(req).await;
/// resp.set_header("x-namix-app", "admin");
///
/// // 或链式
/// next.run(req).await.with_header("x-namix-app", "admin")
/// ```
pub struct Response {
    inner: HttpResponse<Body>,
}

impl Response {
    /// `content_type` 可为 [`ContentType`] 枚举或 `&str` / `String`。
    pub fn new(
        status: StatusCode,
        content_type: impl Into<ContentType>,
        body: impl Into<Bytes>,
    ) -> Self {
        let ct = content_type.into();
        let mut response = HttpResponse::new(body_full(body));
        *response.status_mut() = status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(ct.as_str())
                .unwrap_or_else(|_| HeaderValue::from_static("text/plain; charset=utf-8")),
        );
        Self { inner: response }
    }

    /// 自定义已装箱的 body（SSE 流等）。
    pub(crate) fn from_body(status: StatusCode, body: Body) -> Self {
        let mut response = HttpResponse::new(body);
        *response.status_mut() = status;
        Self { inner: response }
    }

    // ── Status ────────────────────────────────────────────────

    pub fn status(&self) -> StatusCode {
        self.inner.status()
    }

    pub fn set_status(&mut self, status: StatusCode) -> &mut Self {
        *self.inner.status_mut() = status;
        self
    }

    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.set_status(status);
        self
    }

    // ── Header ────────────────────────────────────────────────

    pub fn headers(&self) -> &HeaderMap {
        self.inner.headers()
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        self.inner.headers_mut()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.inner.headers().get(name).and_then(|v| v.to_str().ok())
    }

    pub fn set_header(&mut self, name: &str, value: impl AsRef<str>) -> &mut Self {
        if let (Ok(name), Ok(value)) = (
            HeaderName::try_from(name),
            HeaderValue::from_str(value.as_ref()),
        ) {
            self.inner.headers_mut().insert(name, value);
        }
        self
    }

    pub fn with_header(mut self, name: &str, value: impl AsRef<str>) -> Self {
        self.set_header(name, value);
        self
    }

    pub fn remove_header(&mut self, name: &str) -> &mut Self {
        self.inner.headers_mut().remove(name);
        self
    }

    /// 追加 `Set-Cookie`（可多次调用）。
    pub fn set_cookie(&mut self, name: &str, value: &str) -> &mut Self {
        self.set_cookie_with_options(name, value, CookieOptions::default())
    }

    pub fn set_cookie_with_options(
        &mut self,
        name: &str,
        value: &str,
        options: CookieOptions,
    ) -> &mut Self {
        let mut raw = format!("{name}={value}; Path={}", options.path);
        if let Some(max_age) = options.max_age {
            raw.push_str(&format!("; Max-Age={max_age}"));
        }
        if options.http_only {
            raw.push_str("; HttpOnly");
        }
        if options.secure {
            raw.push_str("; Secure");
        }
        raw.push_str("; SameSite=");
        raw.push_str(options.same_site);
        if let Ok(value) = HeaderValue::from_str(&raw) {
            self.inner.headers_mut().append(header::SET_COOKIE, value);
        }
        self
    }

    pub fn with_cookie(mut self, name: &str, value: &str) -> Self {
        self.set_cookie(name, value);
        self
    }

    pub fn with_cookie_options(mut self, name: &str, value: &str, options: CookieOptions) -> Self {
        self.set_cookie_with_options(name, value, options);
        self
    }

    /// 清除 cookie（浏览器侧立即过期）。
    pub fn clear_cookie(&mut self, name: &str) -> &mut Self {
        self.clear_cookie_with_options(name, CookieOptions::default())
    }

    pub fn clear_cookie_with_options(
        &mut self,
        name: &str,
        mut options: CookieOptions,
    ) -> &mut Self {
        options.max_age = Some(0);
        self.set_cookie_with_options(name, "", options)
    }

    pub fn with_clear_cookie_options(mut self, name: &str, options: CookieOptions) -> Self {
        self.clear_cookie_with_options(name, options);
        self
    }

    /// Backwards-compatible chainable cookie removal.
    pub fn with_clear_cookie(mut self, name: &str) -> Self {
        self.clear_cookie(name);
        self
    }

    /// 附带 error flash（下一请求 `req.flash().error`）。
    pub fn with_flash_error(self, msg: impl AsRef<str>) -> Self {
        self.with_cookie(
            crate::core::controller::FLASH_COOKIE,
            &crate::core::controller::flash_cookie_error(msg.as_ref()),
        )
    }

    /// 附带 success flash（下一请求 `req.flash().success`）。
    pub fn with_flash_ok(self) -> Self {
        self.with_cookie(crate::core::controller::FLASH_COOKIE, "ok")
    }

    /// 消费 flash cookie（页面读完后清掉）。
    pub fn clear_flash(self) -> Self {
        self.with_clear_cookie(crate::core::controller::FLASH_COOKIE)
    }

    /// 若请求带了 flash cookie，则清掉（避免无 flash 时也写 Set-Cookie）。
    pub fn consume_flash(self, req: &crate::core::request::Request) -> Self {
        if req.cookie(crate::core::controller::FLASH_COOKIE).is_some() {
            self.clear_flash()
        } else {
            self
        }
    }

    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    pub fn set_content_type(&mut self, value: impl Into<ContentType>) -> &mut Self {
        let ct = value.into();
        self.set_header("content-type", ct.as_str())
    }

    pub fn with_content_type(mut self, value: impl Into<ContentType>) -> Self {
        self.set_content_type(value);
        self
    }

    // ── Body ──────────────────────────────────────────────────

    pub fn set_body(&mut self, body: impl Into<Bytes>) -> &mut Self {
        *self.inner.body_mut() = body_full(body);
        self
    }

    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.set_body(body);
        self
    }

    // ── File / download ───────────────────────────────────────

    /// 从磁盘读文件并**内联**返回（浏览器可预览 `.md` / 图片等）。
    ///
    /// 对应 Laravel `response()->file($path)` 的语义，路径非法或读失败 → 404。
    pub fn file(path: impl AsRef<Path>) -> Self {
        match read_path(path.as_ref()) {
            Ok(bytes) => {
                let name = file_name(path.as_ref());
                let ct = ContentType::from_path(path.as_ref());
                Response::new(StatusCode::OK, ct, bytes).with_inline(name)
            }
            Err(()) => file_not_found(),
        }
    }

    /// 强制下载磁盘文件（`Content-Disposition: attachment`）。
    ///
    /// 对应 Laravel `response()->download($path)`；文件名取路径 basename。
    pub fn download(path: impl AsRef<Path>) -> Self {
        let name = file_name(path.as_ref()).to_string();
        Self::download_as(path, &name)
    }

    /// 强制下载，并指定下载文件名。
    pub fn download_as(path: impl AsRef<Path>, filename: &str) -> Self {
        match read_path(path.as_ref()) {
            Ok(bytes) => {
                let ct = ContentType::from_path(path.as_ref());
                Response::new(StatusCode::OK, ct, bytes).with_attachment(filename)
            }
            Err(()) => file_not_found(),
        }
    }

    /// 内存内容强制下载。`content_type` 可为 [`ContentType`] 或字符串。
    pub fn download_data(
        filename: &str,
        content_type: impl Into<ContentType>,
        body: impl Into<Bytes>,
    ) -> Self {
        Response::new(StatusCode::OK, content_type, body).with_attachment(filename)
    }

    /// `Content-Disposition: attachment; filename="…"` —— 强制另存为。
    pub fn with_attachment(self, filename: &str) -> Self {
        self.with_header(
            "content-disposition",
            content_disposition("attachment", filename),
        )
    }

    /// `Content-Disposition: inline; filename="…"` —— 浏览器内打开/预览。
    pub fn with_inline(self, filename: &str) -> Self {
        self.with_header(
            "content-disposition",
            content_disposition("inline", filename),
        )
    }

    // ── Redirect ──────────────────────────────────────────────

    /// 302 临时跳转。
    pub fn redirect(to: impl AsRef<str>) -> Self {
        Self::redirect_with(StatusCode::FOUND, to)
    }

    /// 301 永久跳转。
    pub fn redirect_permanent(to: impl AsRef<str>) -> Self {
        Self::redirect_with(StatusCode::MOVED_PERMANENTLY, to)
    }

    /// 303 See Other（POST 后跳转常用）。
    pub fn redirect_see_other(to: impl AsRef<str>) -> Self {
        Self::redirect_with(StatusCode::SEE_OTHER, to)
    }

    pub fn redirect_with(status: StatusCode, to: impl AsRef<str>) -> Self {
        let to = to.as_ref();
        let mut resp = Response::new(status, ContentType::Text, format!("Redirecting to {to}"));
        resp.set_header("location", to);
        resp
    }

    pub fn location(&self) -> Option<&str> {
        self.header("location")
    }

    pub(crate) fn into_inner(self) -> HttpResponse<Body> {
        self.inner
    }

    /// 收集完整 body（HTTP/3、单测）。流式 SSE 会读到结束或断开为止。
    pub(crate) async fn into_status_headers_body(self) -> (StatusCode, HeaderMap, Bytes) {
        let (parts, body) = self.inner.into_parts();
        let bytes = match body.collect().await {
            Ok(c) => c.to_bytes(),
            Err(_) => Bytes::new(),
        };
        (parts.status, parts.headers, bytes)
    }

    pub(crate) fn from_parts(
        status: StatusCode,
        headers: HeaderMap,
        body: impl Into<Bytes>,
    ) -> Self {
        let mut response = HttpResponse::new(body_full(body));
        *response.status_mut() = status;
        *response.headers_mut() = headers;
        Self { inner: response }
    }
}

/// 处理器返回值统一转成 [`Response`]（不依赖 Request）。
pub trait IntoResponse {
    fn into_response(self) -> Response;
}

/// 处理器返回值 → [`Response`]（可带上当前 [`Request`]）。
///
/// 页面直接 `async fn island(req: Request) -> Island`，框架按 `ViewPage` 渲染；
/// 跳转等仍返回 [`Response`]。
pub trait Respond {
    fn respond(self, req: &crate::core::request::Request) -> Response;
}

impl Respond for Response {
    fn respond(self, _req: &crate::core::request::Request) -> Response {
        self
    }
}

impl Respond for String {
    fn respond(self, _req: &crate::core::request::Request) -> Response {
        self.into_response()
    }
}

impl Respond for &'static str {
    fn respond(self, _req: &crate::core::request::Request) -> Response {
        self.into_response()
    }
}

impl Respond for (http::StatusCode, String) {
    fn respond(self, _req: &crate::core::request::Request) -> Response {
        self.into_response()
    }
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK, ContentType::Text, self)
    }
}

impl IntoResponse for &'static str {
    fn into_response(self) -> Response {
        Response::new(StatusCode::OK, ContentType::Text, self)
    }
}

impl IntoResponse for (StatusCode, String) {
    fn into_response(self) -> Response {
        Response::new(self.0, ContentType::Text, self.1)
    }
}

fn read_path(path: &Path) -> Result<Bytes, ()> {
    if path.as_os_str().is_empty() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(());
    }
    std::fs::read(path).map(Bytes::from).map_err(|_| ())
}

fn file_not_found() -> Response {
    Response::new(StatusCode::NOT_FOUND, ContentType::Text, "not found")
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download")
}

fn content_disposition(kind: &str, filename: &str) -> String {
    let safe: String = filename
        .chars()
        .map(|c| {
            if c == '"' || c == '\\' || c == '\r' || c == '\n' {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{kind}; filename=\"{safe}\"")
}
