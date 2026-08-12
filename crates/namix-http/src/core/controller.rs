//! 控制器响应辅助 + 请求上下文 trait（flash / 跳转 / 分段传文件）。
//!
//! 开启 `pages` 后，完整能力在 [`crate::features::pages::Controller`]：
//! `req.view("login").with(...).render()`（Laravel 风格）。
//!
//! ```ignore
//! use namix::prelude::*;
//!
//! pub async fn save(req: Request) -> Response {
//!     match do_save().await {
//!         Ok(()) => req.redirect_ok_to(route::main::me),
//!         Err(e) => req.redirect_error_to(route::main::me, e),
//!     }
//! }
//!
//! // 分段下载 / 断点续传
//! pub async fn video(req: Request) -> Response {
//!     req.serve_file("storage/video.mp4")
//! }
//!
//! // 分段上传
//! pub async fn upload(req: Request) -> Response {
//!     req.upload_chunk("storage/uploads/video.bin")
//! }
//! ```

use std::path::Path;

use http::StatusCode;
use serde::Serialize;

use super::content_type::ContentType;
use super::request::{Request, query_decode_pub, query_encode_pub};
use super::response::Response;
use super::routing::NamedRoute;
use super::transfer;

/// Flash cookie 名（一次读清）。
pub const FLASH_COOKIE: &str = "namix_flash";

/// 一次性提示：优先 cookie，兼容旧 `?error=` / `?ok=1`。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flash {
    pub error: Option<String>,
    pub success: bool,
}

impl Flash {
    pub fn parse_cookie(raw: &str) -> Self {
        let raw = raw.trim();
        if raw == "ok" {
            return Self {
                error: None,
                success: true,
            };
        }
        if let Some(rest) = raw.strip_prefix("e:") {
            return Self {
                error: Some(query_decode_pub(rest)).filter(|s| !s.is_empty()),
                success: false,
            };
        }
        Self::default()
    }
}

/// 控制器上下文：由 [`Request`] 实现。
///
/// 不要求 `impl Controller for MyCtrl`——handler 里直接 `req.redirect_error_to(...)`。
pub trait Controller {
    /// 读取 flash（cookie 优先，其次 query）。
    fn flash(&self) -> Flash;

    /// 303 到命名路由，并带 error flash。
    fn redirect_error_to<R: NamedRoute>(&self, route: R, msg: impl AsRef<str>) -> Response;

    /// 303 到命名路由，并带 success flash。
    fn redirect_ok_to<R: NamedRoute>(&self, route: R) -> Response;

    /// 303 到命名路由（无 flash）。
    fn see_other_to<R: NamedRoute>(&self, route: R) -> Response;

    /// 303 到任意 URL。
    fn see_other(&self, to: impl AsRef<str>) -> Response;

    // ── 分段下载 / 断点续传 ─────────────────────────────────

    /// 内联提供文件，自动处理 `Range`（200 整文件 / 206 分段 / 416）。
    ///
    /// 响应带 `Accept-Ranges: bytes`，客户端可断点续传下载。
    fn serve_file(&self, path: impl AsRef<Path>) -> Response;

    /// 强制下载文件，同样支持 `Range` 断点续传。
    fn serve_download(&self, path: impl AsRef<Path>) -> Response;

    /// 强制下载并指定文件名，支持 `Range`。
    fn serve_download_as(&self, path: impl AsRef<Path>, filename: &str) -> Response;

    // ── 分段上传 / 断点续传 ─────────────────────────────────

    /// 接收一块上传数据。
    ///
    /// 请求头：`Content-Range: bytes {start}-{end}/{total}`，body 为该段字节。
    /// 成功：`200`（未完成）或 `201`（完成），JSON + `Upload-Offset` 头。
    /// 无 `Content-Range` 时按整文件写入。
    fn upload_chunk(&self, path: impl AsRef<Path>) -> Response;

    /// 查询已上传偏移（断点续传：客户端读 `Upload-Offset` / JSON `offset` 后续传）。
    fn upload_offset(&self, path: impl AsRef<Path>) -> Response;
}

impl Controller for Request {
    fn flash(&self) -> Flash {
        if let Some(raw) = self.cookie(FLASH_COOKIE) {
            let opened = crate::core::crypt::open_value(raw);
            return Flash::parse_cookie(&opened);
        }
        let error = self
            .query("error")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let success = matches!(
            self.query("ok").as_deref().map(str::trim),
            Some("1") | Some("true") | Some("yes")
        );
        Flash { error, success }
    }

    fn redirect_error_to<R: NamedRoute>(&self, route: R, msg: impl AsRef<str>) -> Response {
        match self.url(route) {
            Some(url) => Response::redirect_see_other(url).with_flash_error(msg),
            None => Response::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ContentType::Text,
                "unknown route for redirect_error_to",
            ),
        }
    }

    fn redirect_ok_to<R: NamedRoute>(&self, route: R) -> Response {
        match self.url(route) {
            Some(url) => Response::redirect_see_other(url).with_flash_ok(),
            None => Response::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ContentType::Text,
                "unknown route for redirect_ok_to",
            ),
        }
    }

    fn see_other_to<R: NamedRoute>(&self, route: R) -> Response {
        match self.url(route) {
            Some(url) => Response::redirect_see_other(url),
            None => Response::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                ContentType::Text,
                "unknown route for see_other_to",
            ),
        }
    }

    fn see_other(&self, to: impl AsRef<str>) -> Response {
        Response::redirect_see_other(to)
    }

    fn serve_file(&self, path: impl AsRef<Path>) -> Response {
        transfer::serve_path(self, path, false, None)
    }

    fn serve_download(&self, path: impl AsRef<Path>) -> Response {
        transfer::serve_path(self, path, true, None)
    }

    fn serve_download_as(&self, path: impl AsRef<Path>, filename: &str) -> Response {
        transfer::serve_path(self, path, true, Some(filename))
    }

    fn upload_chunk(&self, path: impl AsRef<Path>) -> Response {
        transfer::receive_chunk(self, path)
    }

    fn upload_offset(&self, path: impl AsRef<Path>) -> Response {
        transfer::upload_status(path)
    }
}

/// `text/plain` —— `text("ok")` / `text(format!("user {}", id))`
pub fn text(body: impl Into<String>) -> Response {
    Response::new(StatusCode::OK, ContentType::Text, body.into())
}

/// `text/html`
pub fn html(body: impl Into<String>) -> Response {
    Response::new(StatusCode::OK, ContentType::Html, body.into())
}

/// `application/json` —— 传入可序列化值（结构体 / `json!({...})`）。
///
/// ```ignore
/// json(json!({ "token": jwt, "user": { "id": 1 } }))
/// json(UserDto { id: 1, name: "a".into() })
/// ```
pub fn json(value: impl Serialize) -> Response {
    match serde_json::to_string(&value) {
        Ok(body) => json_raw(body),
        Err(e) => Response::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ContentType::Json,
            format!(r#"{{"error":"json serialize failed: {e}"}}"#),
        ),
    }
}

/// 已是 JSON 字符串时用（避免再被 `Serialize` 包一层引号）。
pub fn json_raw(body: impl Into<String>) -> Response {
    Response::new(StatusCode::OK, ContentType::Json, body.into())
}

/// 任意 Content-Type：枚举或字符串均可。
///
/// ```ignore
/// raw(ContentType::Markdown, "# hi")
/// raw("application/vnd.api+json", body)
/// ```
pub fn raw(content_type: impl Into<ContentType>, body: impl Into<String>) -> Response {
    Response::new(StatusCode::OK, content_type, body.into())
}

/// 磁盘文件内联预览（**不**处理 `Range`）。要断点续传请用 `req.serve_file(...)`。
pub fn file(path: impl AsRef<std::path::Path>) -> Response {
    Response::file(path)
}

/// 磁盘文件强制下载（**不**处理 `Range`）。要断点续传请用 `req.serve_download(...)`。
pub fn download(path: impl AsRef<std::path::Path>) -> Response {
    Response::download(path)
}

/// 磁盘文件强制下载并改名 —— `download_as("docs/a.md", "guide.md")`。
pub fn download_as(path: impl AsRef<std::path::Path>, filename: &str) -> Response {
    Response::download_as(path, filename)
}

/// 内存内容强制下载 —— `download_data("export.md", ContentType::Markdown, body)`。
pub fn download_data(
    filename: &str,
    content_type: impl Into<ContentType>,
    body: impl Into<bytes::Bytes>,
) -> Response {
    Response::download_data(filename, content_type, body)
}

pub fn with_status(status: StatusCode, body: impl Into<String>) -> Response {
    Response::new(status, ContentType::Text, body.into())
}

pub fn not_found() -> Response {
    with_status(StatusCode::NOT_FOUND, "not found")
}

/// 204 No Content
pub fn no_content() -> Response {
    Response::new(StatusCode::NO_CONTENT, ContentType::Text, "")
}

/// 302 跳转（控制器 / 中间件均可）。
pub fn redirect(to: impl AsRef<str>) -> Response {
    Response::redirect(to)
}

pub fn redirect_permanent(to: impl AsRef<str>) -> Response {
    Response::redirect_permanent(to)
}

pub(crate) fn flash_cookie_error(msg: &str) -> String {
    let plain = format!("e:{}", query_encode_pub(msg));
    crate::core::crypt::seal_value(&plain)
}

pub(crate) fn flash_cookie_ok() -> String {
    crate::core::crypt::seal_value("ok")
}
