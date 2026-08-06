//! HTTP `Content-Type`：常用值用枚举，任意字符串也能传。
//!
//! ```ignore
//! use namix::prelude::*;
//!
//! // 枚举（文本类自带 charset=utf-8）
//! raw(ContentType::Markdown, "# hi")
//! download_data("a.md", ContentType::Markdown, "# hi")
//!
//! // 字符串同样可以
//! raw("text/markdown; charset=utf-8", "# hi")
//! Response::new(StatusCode::OK, ContentType::Png, bytes)
//! ```

use std::fmt;
use std::path::Path;

/// 响应 `Content-Type`。
///
/// - **常用类型**：用枚举变体，省得手写 MIME 字符串。
/// - **文本类**（Text / Markdown / Html / Css / Js / Json / Xml）：默认带 `charset=utf-8`。
/// - **二进制**（图 / 音视频 / PDF 等）：不加 charset。
/// - **自定义**：[`ContentType::Custom`] 或直接传 `&str`（经 [`From`]）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContentType {
    // ── 文本 ──────────────────────────────────────────────
    /// `text/plain; charset=utf-8`
    Text,
    /// `text/markdown; charset=utf-8`
    Markdown,
    /// `text/html; charset=utf-8`
    Html,
    /// `text/css; charset=utf-8`
    Css,
    /// `application/javascript; charset=utf-8`
    Javascript,
    /// `application/json; charset=utf-8`
    Json,
    /// `application/xml; charset=utf-8`
    Xml,

    // ── 图片 ──────────────────────────────────────────────
    /// `image/png`
    Png,
    /// `image/jpeg`
    Jpeg,
    /// `image/gif`
    Gif,
    /// `image/webp`
    Webp,
    /// `image/svg+xml`
    Svg,

    // ── 音视频 ────────────────────────────────────────────
    /// `video/mp4`
    Mp4,
    /// `video/webm`
    Webm,
    /// `video/quicktime`（`.mov`）
    Mov,
    /// `audio/mpeg`（`.mp3`）
    Mp3,
    /// `audio/wav`
    Wav,

    // ── 其它常见 ──────────────────────────────────────────
    /// `application/pdf`
    Pdf,
    /// `application/zip`
    Zip,
    /// `font/woff2`
    Woff2,
    /// `application/wasm`（WebAssembly）
    Wasm,
    /// `application/octet-stream`（未知二进制）
    OctetStream,

    /// 完整自定义值，例如 `application/vnd.api+json`。
    /// 也可用 `ContentType::from("...")` / `raw("...", body)`。
    Custom(String),
}

impl ContentType {
    /// 写入响应头的完整字符串（含常用文本类型的 `charset=utf-8`）。
    pub fn as_str(&self) -> &str {
        match self {
            Self::Text => "text/plain; charset=utf-8",
            Self::Markdown => "text/markdown; charset=utf-8",
            Self::Html => "text/html; charset=utf-8",
            Self::Css => "text/css; charset=utf-8",
            Self::Javascript => "application/javascript; charset=utf-8",
            Self::Json => "application/json; charset=utf-8",
            Self::Xml => "application/xml; charset=utf-8",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Svg => "image/svg+xml",
            Self::Mp4 => "video/mp4",
            Self::Webm => "video/webm",
            Self::Mov => "video/quicktime",
            Self::Mp3 => "audio/mpeg",
            Self::Wav => "audio/wav",
            Self::Pdf => "application/pdf",
            Self::Zip => "application/zip",
            Self::Woff2 => "font/woff2",
            Self::Wasm => "application/wasm",
            Self::OctetStream => "application/octet-stream",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// 自定义完整 `Content-Type` 字符串。
    pub fn custom(value: impl Into<String>) -> Self {
        Self::Custom(value.into())
    }

    /// 按文件扩展名猜测类型；未知则 [`ContentType::OctetStream`]。
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        path.as_ref()
            .extension()
            .and_then(|e| e.to_str())
            .map(Self::from_ext)
            .unwrap_or(Self::OctetStream)
    }

    /// 按扩展名（不含点，大小写不敏感）猜测。
    pub fn from_ext(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "txt" => Self::Text,
            "md" | "markdown" => Self::Markdown,
            "html" | "htm" => Self::Html,
            "css" => Self::Css,
            "js" | "mjs" => Self::Javascript,
            "json" | "map" => Self::Json,
            "xml" => Self::Xml,
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpeg,
            "gif" => Self::Gif,
            "webp" => Self::Webp,
            "svg" => Self::Svg,
            "mp4" => Self::Mp4,
            "webm" => Self::Webm,
            "mov" => Self::Mov,
            "mp3" => Self::Mp3,
            "wav" => Self::Wav,
            "pdf" => Self::Pdf,
            "zip" => Self::Zip,
            "woff2" => Self::Woff2,
            "wasm" => Self::Wasm,
            _ => Self::OctetStream,
        }
    }

    /// 是否为文本类（枚举里会带 `charset=utf-8` 的那一类）。
    pub fn is_text(&self) -> bool {
        matches!(
            self,
            Self::Text
                | Self::Markdown
                | Self::Html
                | Self::Css
                | Self::Javascript
                | Self::Json
                | Self::Xml
        )
    }
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl AsRef<str> for ContentType {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for ContentType {
    /// 字符串 → 枚举：能识别的 MIME（可带/不带 charset）收成变体，否则 [`ContentType::Custom`]。
    fn from(value: &str) -> Self {
        let trimmed = value.trim();
        let base = trimmed
            .split(';')
            .next()
            .unwrap_or(trimmed)
            .trim()
            .to_ascii_lowercase();
        match base.as_str() {
            "text/plain" => Self::Text,
            "text/markdown" => Self::Markdown,
            "text/html" => Self::Html,
            "text/css" => Self::Css,
            "application/javascript" | "text/javascript" => Self::Javascript,
            "application/json" => Self::Json,
            "application/xml" | "text/xml" => Self::Xml,
            "image/png" => Self::Png,
            "image/jpeg" => Self::Jpeg,
            "image/gif" => Self::Gif,
            "image/webp" => Self::Webp,
            "image/svg+xml" => Self::Svg,
            "video/mp4" => Self::Mp4,
            "video/webm" => Self::Webm,
            "video/quicktime" => Self::Mov,
            "audio/mpeg" => Self::Mp3,
            "audio/wav" | "audio/x-wav" => Self::Wav,
            "application/pdf" => Self::Pdf,
            "application/zip" => Self::Zip,
            "font/woff2" => Self::Woff2,
            "application/wasm" => Self::Wasm,
            "application/octet-stream" => Self::OctetStream,
            _ => Self::Custom(trimmed.to_string()),
        }
    }
}

impl From<String> for ContentType {
    fn from(value: String) -> Self {
        ContentType::from(value.as_str())
    }
}

impl From<&String> for ContentType {
    fn from(value: &String) -> Self {
        ContentType::from(value.as_str())
    }
}
