//! 分段上传 / 分段下载 / 断点续传（HTTP Range）。
//!
//! ## 下载（断点续传）
//!
//! 客户端带 `Range: bytes=start-end`，服务端回 **206** + `Content-Range` + `Accept-Ranges: bytes`。
//! 无 Range 时回整文件 **200**（仍带 `Accept-Ranges`，便于后续续传）。
//!
//! ## 上传（分段 + 断点续传）
//!
//! 客户端按块 `PUT`/`POST`，带：
//! ```text
//! Content-Range: bytes {start}-{end}/{total}
//! ```
//! 服务端按偏移写入同一文件；响应带 `Upload-Offset`（已写入的下一字节位置）。
//! 查询进度：不带 body，或 `Content-Range: bytes */{total}`，用 [`upload_offset`](crate::core::controller::Controller::upload_offset)。
//!
//! ```ignore
//! // 下载（自动处理 Range）
//! req.serve_file("videos/a.mp4")
//! req.serve_download("docs/a.pdf")
//!
//! // 上传一块
//! req.upload_chunk("uploads/video.bin")
//!
//! // 查询已上传偏移（客户端据此续传）
//! req.upload_offset("uploads/video.bin")
//! ```

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path};

use bytes::Bytes;
use http::StatusCode;
use serde::Serialize;

use super::content_type::ContentType;
use super::request::Request;
use super::response::Response;

/// 闭区间字节范围：`start..=end`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

impl ByteRange {
    pub fn is_empty(self) -> bool {
        self.start > self.end
    }

    pub fn len(self) -> u64 {
        if self.is_empty() {
            0
        } else {
            self.end - self.start + 1
        }
    }
}

/// 上传用 `Content-Range: bytes start-end/total`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    pub start: u64,
    pub end: u64,
    pub total: u64,
}

impl ContentRange {
    pub fn is_empty(self) -> bool {
        self.start > self.end
    }

    pub fn len(self) -> u64 {
        if self.is_empty() {
            0
        } else {
            self.end - self.start + 1
        }
    }
}

/// `Content-Range` 头：数据段，或 `bytes */total`（仅总量 / 查询）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentRangeHeader {
    /// `bytes */total`
    TotalOnly { total: u64 },
    /// `bytes start-end/total`
    Span(ContentRange),
}

/// 一次分段上传后的进度（也用于 JSON 响应）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgress {
    /// 下一字节应从哪个 offset 传（已连续写入的长度）。
    pub offset: u64,
    /// 声明的总大小（若已知）。
    pub total: Option<u64>,
    /// 是否已收齐。
    pub complete: bool,
}

/// `Range` 头的解析错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeParseError {
    UnsupportedUnit,
    MultipleRanges,
    InvalidSyntax,
    Unsatisfiable,
}

impl fmt::Display for RangeParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedUnit => "Range unit must be bytes",
            Self::MultipleRanges => "multiple byte ranges are not supported",
            Self::InvalidSyntax => "invalid byte range syntax",
            Self::Unsatisfiable => "byte range is not satisfiable",
        };
        f.write_str(message)
    }
}

impl std::error::Error for RangeParseError {}

/// 解析 `Range: bytes=0-1023` / `bytes=0-` / `bytes=-500`（仅支持**单段**）。
pub fn parse_range_header(header: &str, file_size: u64) -> Result<ByteRange, RangeParseError> {
    let header = header.trim();
    let Some(spec) = header.strip_prefix("bytes=") else {
        return Err(RangeParseError::UnsupportedUnit);
    };
    // 多段暂不支持
    if spec.contains(',') {
        return Err(RangeParseError::MultipleRanges);
    }
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(RangeParseError::InvalidSyntax);
    }

    if let Some(suffix) = spec.strip_prefix('-') {
        // bytes=-500 → 最后 500 字节
        let n: u64 = suffix.parse().map_err(|_| RangeParseError::InvalidSyntax)?;
        if n == 0 || file_size == 0 {
            return Err(RangeParseError::Unsatisfiable);
        }
        let n = n.min(file_size);
        return Ok(ByteRange {
            start: file_size - n,
            end: file_size - 1,
        });
    }

    let (start_s, end_s) = spec.split_once('-').ok_or(RangeParseError::InvalidSyntax)?;
    let start: u64 = start_s
        .parse()
        .map_err(|_| RangeParseError::InvalidSyntax)?;
    if start >= file_size {
        return Err(RangeParseError::Unsatisfiable);
    }
    let end = if end_s.is_empty() {
        file_size - 1
    } else {
        let end: u64 = end_s.parse().map_err(|_| RangeParseError::InvalidSyntax)?;
        end.min(file_size - 1)
    };
    if end < start {
        return Err(RangeParseError::Unsatisfiable);
    }
    Ok(ByteRange { start, end })
}

/// 解析 `Content-Range: bytes 0-1023/5000` 或 `bytes */5000`。
pub fn parse_content_range(header: &str) -> Result<ContentRangeHeader, String> {
    let header = header.trim();
    let rest = header
        .strip_prefix("bytes ")
        .or_else(|| header.strip_prefix("bytes="))
        .ok_or_else(|| "Content-Range must start with 'bytes '".to_string())?
        .trim();

    let (range_part, total_part) = rest
        .split_once('/')
        .ok_or_else(|| "Content-Range missing /total".to_string())?;
    let total: u64 = total_part
        .trim()
        .parse()
        .map_err(|_| "invalid Content-Range total".to_string())?;
    if total == 0 {
        return Err("Content-Range total must be > 0".into());
    }

    let range_part = range_part.trim();
    if range_part == "*" {
        return Ok(ContentRangeHeader::TotalOnly { total });
    }

    let (start_s, end_s) = range_part
        .split_once('-')
        .ok_or_else(|| "Content-Range missing start-end".to_string())?;
    let start: u64 = start_s
        .trim()
        .parse()
        .map_err(|_| "invalid Content-Range start".to_string())?;
    let end: u64 = end_s
        .trim()
        .parse()
        .map_err(|_| "invalid Content-Range end".to_string())?;
    if end < start {
        return Err("Content-Range end < start".into());
    }
    if end >= total {
        return Err("Content-Range end >= total".into());
    }
    Ok(ContentRangeHeader::Span(ContentRange { start, end, total }))
}

#[derive(Clone, Copy)]
enum Disposition {
    Inline,
    Attachment,
}

/// 按请求 `Range` 提供文件（200 / 206 / 416）。
pub fn serve_path(
    req: &Request,
    path: impl AsRef<Path>,
    download: bool,
    filename: Option<&str>,
) -> Response {
    let path = path.as_ref();
    if !is_safe_path(path) {
        return not_found();
    }
    let meta = match fs::metadata(path) {
        Ok(m) if m.is_file() => m,
        _ => return not_found(),
    };
    let file_size = meta.len();
    let ct = ContentType::from_path(path);
    let name = filename.unwrap_or_else(|| {
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("download")
    });
    let disp = if download {
        Disposition::Attachment
    } else {
        Disposition::Inline
    };

    let range = match req.header("range") {
        Some(h) => match parse_range_header(h, file_size) {
            Ok(r) => Some(r),
            Err(_) => {
                return Response::new(StatusCode::RANGE_NOT_SATISFIABLE, ct, Bytes::new())
                    .with_header("accept-ranges", "bytes")
                    .with_header("content-range", format!("bytes */{file_size}"))
                    .with_disposition(disp, name);
            }
        },
        None => None,
    };

    match range {
        None => match read_file_all(path) {
            Ok(bytes) => Response::new(StatusCode::OK, ct, bytes)
                .with_header("accept-ranges", "bytes")
                .with_header("content-length", file_size.to_string())
                .with_disposition(disp, name),
            Err(_) => not_found(),
        },
        Some(r) => match read_file_range(path, r) {
            Ok(bytes) => Response::new(StatusCode::PARTIAL_CONTENT, ct, bytes)
                .with_header("accept-ranges", "bytes")
                .with_header(
                    "content-range",
                    format!("bytes {}-{}/{}", r.start, r.end, file_size),
                )
                .with_header("content-length", r.len().to_string())
                .with_disposition(disp, name),
            Err(_) => not_found(),
        },
    }
}

/// 将当前请求 body 按 `Content-Range` **顺序追加**写入 `path`（分段上传）。
///
/// 模型与 tus 类似：只接受 `start == 当前文件长度` 的下一块，保证 `Upload-Offset` 可靠。
pub fn receive_chunk(req: &Request, path: impl AsRef<Path>) -> Response {
    let path = path.as_ref();
    if !is_safe_path(path) {
        return bad_request("unsafe path");
    }

    let header = match req.header("content-range") {
        Some(h) => h,
        None => return receive_whole(req, path),
    };

    let parsed = match parse_content_range(header) {
        Ok(v) => v,
        Err(e) => return bad_request(e),
    };

    match parsed {
        ContentRangeHeader::TotalOnly { total } => {
            let _ = write_total_hint(path, total);
            upload_status_response(path, Some(total))
        }
        ContentRangeHeader::Span(cr) => write_span(req, path, cr),
    }
}

fn write_span(req: &Request, path: &Path, cr: ContentRange) -> Response {
    let body = req.body();
    if body.len() as u64 != cr.len() {
        return bad_request(format!(
            "body length {} != Content-Range length {}",
            body.len(),
            cr.len()
        ));
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent)
    {
        return server_error(format!("create_dir: {e}"));
    }

    let current = file_len(path);
    if cr.start != current {
        // 409：偏移不对，告诉客户端应从哪续传
        let progress = UploadProgress {
            offset: current,
            total: Some(cr.total),
            complete: current >= cr.total && cr.total > 0,
        };
        return match serde_json::to_string(&progress) {
            Ok(body) => Response::new(StatusCode::CONFLICT, ContentType::Json, body)
                .with_header("upload-offset", current.to_string())
                .with_header("accept-ranges", "bytes")
                .with_header("cache-control", "no-store"),
            Err(e) => server_error(e.to_string()),
        };
    }

    let mut file = match OpenOptions::new().create(true).append(true).open(path) {
        Ok(f) => f,
        Err(e) => return server_error(format!("open: {e}")),
    };

    if let Err(e) = file.write_all(body) {
        return server_error(format!("write: {e}"));
    }
    if let Err(e) = file.flush() {
        return server_error(format!("flush: {e}"));
    }

    // 记总量，便于 upload_offset 判断 complete
    let _ = write_total_hint(path, cr.total);

    let offset = file_len(path);
    let complete = offset >= cr.total;
    upload_progress_response(UploadProgress {
        offset: offset.min(cr.total),
        total: Some(cr.total),
        complete,
    })
}

/// 查询已上传偏移（断点续传：客户端用 `Upload-Offset` 继续传）。
pub fn upload_status(path: impl AsRef<Path>) -> Response {
    let path = path.as_ref();
    if !is_safe_path(path) {
        return not_found();
    }
    upload_status_response(path, None)
}

fn receive_whole(req: &Request, path: &Path) -> Response {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(e) = fs::create_dir_all(parent)
    {
        return server_error(format!("create_dir: {e}"));
    }
    let body = req.body();
    match fs::write(path, body) {
        Ok(()) => {
            let len = body.len() as u64;
            upload_progress_response(UploadProgress {
                offset: len,
                total: Some(len),
                complete: true,
            })
        }
        Err(e) => server_error(format!("write: {e}")),
    }
}

fn upload_status_response(path: &Path, total_hint: Option<u64>) -> Response {
    let offset = file_len(path);
    let total = total_hint.or_else(|| read_total_hint(path));
    let complete = match total {
        Some(t) => offset >= t && t > 0,
        None => false,
    };
    upload_progress_response(UploadProgress {
        offset,
        total,
        complete,
    })
}

fn upload_progress_response(progress: UploadProgress) -> Response {
    // 未完成 200、完成 201；均带 Upload-Offset，便于断点续传。
    let status = if progress.complete {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    match serde_json::to_string(&progress) {
        Ok(body) => Response::new(status, ContentType::Json, body)
            .with_header("upload-offset", progress.offset.to_string())
            .with_header("accept-ranges", "bytes")
            .with_header("cache-control", "no-store"),
        Err(e) => server_error(e.to_string()),
    }
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn total_hint_path(path: &Path) -> std::path::PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(".namix-total");
    std::path::PathBuf::from(p)
}

fn write_total_hint(path: &Path, total: u64) -> std::io::Result<()> {
    fs::write(total_hint_path(path), total.to_string())
}

fn read_total_hint(path: &Path) -> Option<u64> {
    fs::read_to_string(total_hint_path(path))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

fn read_file_all(path: &Path) -> std::io::Result<Bytes> {
    Ok(Bytes::from(fs::read(path)?))
}

fn read_file_range(path: &Path, range: ByteRange) -> std::io::Result<Bytes> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(range.start))?;
    let mut buf = vec![0u8; range.len() as usize];
    file.read_exact(&mut buf)?;
    Ok(Bytes::from(buf))
}

fn is_safe_path(path: &Path) -> bool {
    !path.as_os_str().is_empty() && !path.components().any(|c| matches!(c, Component::ParentDir))
}

fn not_found() -> Response {
    Response::new(StatusCode::NOT_FOUND, ContentType::Text, "not found")
}

fn bad_request(msg: impl Into<String>) -> Response {
    Response::new(StatusCode::BAD_REQUEST, ContentType::Text, msg.into())
}

fn server_error(msg: impl Into<String>) -> Response {
    Response::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        ContentType::Text,
        msg.into(),
    )
}

trait WithDisposition {
    fn with_disposition(self, disp: Disposition, name: &str) -> Self;
}

impl WithDisposition for Response {
    fn with_disposition(self, disp: Disposition, name: &str) -> Self {
        match disp {
            Disposition::Inline => self.with_inline(name),
            Disposition::Attachment => self.with_attachment(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_suffix_and_open_end() {
        assert_eq!(
            parse_range_header("bytes=0-99", 1000).unwrap(),
            ByteRange { start: 0, end: 99 }
        );
        assert_eq!(
            parse_range_header("bytes=100-", 1000).unwrap(),
            ByteRange {
                start: 100,
                end: 999
            }
        );
        assert_eq!(
            parse_range_header("bytes=-100", 1000).unwrap(),
            ByteRange {
                start: 900,
                end: 999
            }
        );
    }

    #[test]
    fn content_range_parse() {
        assert_eq!(
            parse_content_range("bytes 0-1023/5000").unwrap(),
            ContentRangeHeader::Span(ContentRange {
                start: 0,
                end: 1023,
                total: 5000
            })
        );
        assert_eq!(
            parse_content_range("bytes */5000").unwrap(),
            ContentRangeHeader::TotalOnly { total: 5000 }
        );
    }
}
