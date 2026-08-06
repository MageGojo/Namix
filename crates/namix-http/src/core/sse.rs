//! Server-Sent Events（SSE）。
//!
//! ```ignore
//! use namix::prelude::*;
//!
//! pub async fn ticks(_req: Request) -> Response {
//!     let (sse, tx) = Sse::channel();
//!     tokio::spawn(async move {
//!         for i in 1..=5 {
//!             if tx.send(SseEvent::data(format!("tick {i}"))).await.is_err() {
//!                 break; // 客户端断开
//!             }
//!             tokio::time::sleep(std::time::Duration::from_secs(1)).await;
//!         }
//!     });
//!     sse.into_response()
//! }
//! ```

use std::convert::Infallible;
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use http::StatusCode;
use http_body::Frame;
use http_body_util::{BodyExt, StreamBody};
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::response::{Body, IntoResponse, Response};

/// 一条 SSE 事件（`text/event-stream` 字段）。
#[derive(Debug, Clone, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub id: Option<String>,
    pub retry: Option<Duration>,
    pub data: String,
}

impl SseEvent {
    pub fn data(data: impl Into<String>) -> Self {
        Self {
            data: data.into(),
            ..Self::default()
        }
    }

    pub fn json<T: Serialize>(value: &T) -> Self {
        Self::data(serde_json::to_string(value).unwrap_or_else(|_| "{}".into()))
    }

    pub fn event(mut self, name: impl Into<String>) -> Self {
        self.event = Some(name.into());
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn retry(mut self, d: Duration) -> Self {
        self.retry = Some(d);
        self
    }

    /// 编码为 SSE 文本帧（含结尾空行）。
    pub fn encode(&self) -> Bytes {
        let mut out = String::new();
        if let Some(id) = &self.id {
            out.push_str("id: ");
            out.push_str(id);
            out.push('\n');
        }
        if let Some(event) = &self.event {
            out.push_str("event: ");
            out.push_str(event);
            out.push('\n');
        }
        if let Some(retry) = self.retry {
            out.push_str("retry: ");
            out.push_str(&retry.as_millis().to_string());
            out.push('\n');
        }
        for line in self.data.split('\n') {
            out.push_str("data: ");
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
        Bytes::from(out)
    }
}

enum Wire {
    Event(SseEvent),
    /// 已编码字节（注释 keep-alive 等）
    Raw(Bytes),
}

/// 向客户端推事件；`send` 失败表示连接已关。
#[derive(Clone)]
pub struct SseSender {
    tx: mpsc::Sender<Wire>,
}

impl SseSender {
    pub async fn send(&self, event: SseEvent) -> Result<(), SseClosed> {
        self.tx
            .send(Wire::Event(event))
            .await
            .map_err(|_| SseClosed)
    }

    pub fn try_send(&self, event: SseEvent) -> Result<(), SseClosed> {
        self.tx.try_send(Wire::Event(event)).map_err(|_| SseClosed)
    }

    /// 注释行（`: …`），常用于 keep-alive。
    pub async fn comment(&self, text: impl Into<String>) -> Result<(), SseClosed> {
        let mut raw = String::from(": ");
        raw.push_str(&text.into());
        raw.push('\n');
        raw.push('\n');
        self.tx
            .send(Wire::Raw(Bytes::from(raw)))
            .await
            .map_err(|_| SseClosed)
    }
}

#[derive(Debug)]
pub struct SseClosed;

impl std::fmt::Display for SseClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sse client closed")
    }
}

impl std::error::Error for SseClosed {}

/// 可直接 `return sse.into_response()` 的 SSE 响应。
pub struct Sse {
    body: Body,
}

impl Sse {
    /// 创建通道：业务 `spawn` 里 `tx.send`，返回 HTTP 流式响应。
    pub fn channel() -> (Self, SseSender) {
        Self::channel_with_capacity(32)
    }

    pub fn channel_with_capacity(capacity: usize) -> (Self, SseSender) {
        let (tx, rx) = mpsc::channel::<Wire>(capacity);
        let stream = ReceiverStream::new(rx).map(|wire| {
            let bytes = match wire {
                Wire::Event(ev) => ev.encode(),
                Wire::Raw(b) => b,
            };
            Ok::<_, Infallible>(Frame::data(bytes))
        });
        let body = StreamBody::new(stream)
            .map_err(|never| match never {})
            .boxed();
        (Self { body }, SseSender { tx })
    }

    /// 闭包内推流（框架自动 `spawn`）。
    pub fn stream<F, Fut>(f: F) -> Self
    where
        F: FnOnce(SseSender) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (sse, tx) = Self::channel();
        tokio::spawn(f(tx));
        sse
    }
}

impl IntoResponse for Sse {
    fn into_response(self) -> Response {
        let mut resp = Response::from_body(StatusCode::OK, self.body);
        resp.set_header("content-type", "text/event-stream; charset=utf-8");
        resp.set_header("cache-control", "no-cache");
        resp.set_header("connection", "keep-alive");
        resp.set_header("x-accel-buffering", "no");
        resp
    }
}

impl From<Sse> for Response {
    fn from(sse: Sse) -> Self {
        sse.into_response()
    }
}
