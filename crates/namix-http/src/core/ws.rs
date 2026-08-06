//! WebSocket（`ws://`）与 WSS（复用 HTTPS 监听，客户端写 `wss://`）。
//!
//! ```ignore
//! use namix::prelude::*;
//!
//! // 路由
//! Route::ws("/ws/echo", echo).name("ws.echo").register()
//!
//! async fn echo(_req: Request, mut socket: WsSocket) {
//!     while let Some(msg) = socket.recv().await {
//!         if socket.send(msg).await.is_err() {
//!             break;
//!         }
//!     }
//! }
//! ```

use std::future::Future;
use std::sync::Arc;

use base64::Engine;
use bytes::Bytes;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use hyper::body::Incoming;
use hyper::{Request as HyperRequest, Response as HyperResponse};
use hyper_util::rt::TokioIo;
use serde::Serialize;
use sha1::{Digest, Sha1};
use thiserror::Error;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::{Message, Role};

use crate::core::middleware::BoxFuture;
use crate::core::request::Request;
use crate::core::response::{Body, Response, body_full};
use crate::core::routing::path::PathPattern;

/// WebSocket 文本/二进制消息。
#[derive(Debug, Clone)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}

impl WsMessage {
    pub fn text(s: impl Into<String>) -> Self {
        Self::Text(s.into())
    }

    pub fn binary(b: impl Into<Vec<u8>>) -> Self {
        Self::Binary(b.into())
    }
}

type WsStream = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;

/// 已完成握手的套接字。
pub struct WsSocket {
    inner: WsStream,
}

/// 只写半边（聊天广播常用：读/写拆成两个 task）。
pub struct WsSender {
    inner: SplitSink<WsStream, Message>,
}

/// 只读半边。
pub struct WsReceiver {
    inner: SplitStream<WsStream>,
}

impl WsSocket {
    /// 拆成收/发两端，便于 `select!` / 并行推送。
    pub fn split(self) -> (WsSender, WsReceiver) {
        let (sink, stream) = self.inner.split();
        (WsSender { inner: sink }, WsReceiver { inner: stream })
    }

    pub async fn recv(&mut self) -> Option<WsMessage> {
        recv_message(&mut self.inner).await
    }

    pub async fn send(&mut self, msg: WsMessage) -> Result<(), WsError> {
        send_message(&mut self.inner, msg).await
    }

    pub async fn send_text(&mut self, text: impl Into<String>) -> Result<(), WsError> {
        self.send(WsMessage::Text(text.into())).await
    }

    pub async fn send_json<T: Serialize>(&mut self, value: &T) -> Result<(), WsError> {
        let text = serde_json::to_string(value).map_err(WsError::Serialize)?;
        self.send_text(text).await
    }

    pub async fn close(mut self) -> Result<(), WsError> {
        self.send(WsMessage::Close).await
    }
}

impl WsSender {
    pub async fn send(&mut self, msg: WsMessage) -> Result<(), WsError> {
        send_message(&mut self.inner, msg).await
    }

    pub async fn send_text(&mut self, text: impl Into<String>) -> Result<(), WsError> {
        self.send(WsMessage::Text(text.into())).await
    }

    pub async fn send_json<T: Serialize>(&mut self, value: &T) -> Result<(), WsError> {
        let text = serde_json::to_string(value).map_err(WsError::Serialize)?;
        self.send_text(text).await
    }
}

impl WsReceiver {
    pub async fn recv(&mut self) -> Option<WsMessage> {
        recv_message(&mut self.inner).await
    }
}

async fn recv_message<S>(stream: &mut S) -> Option<WsMessage>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match stream.next().await? {
            Ok(Message::Text(t)) => return Some(WsMessage::Text(t.to_string())),
            Ok(Message::Binary(b)) => return Some(WsMessage::Binary(b.to_vec())),
            Ok(Message::Ping(p)) => return Some(WsMessage::Ping(p.to_vec())),
            Ok(Message::Pong(p)) => return Some(WsMessage::Pong(p.to_vec())),
            Ok(Message::Close(_)) => return Some(WsMessage::Close),
            Ok(Message::Frame(_)) => continue,
            Err(err) => {
                eprintln!("websocket recv error: {err}");
                return None;
            }
        }
    }
}

async fn send_message<S>(sink: &mut S, msg: WsMessage) -> Result<(), WsError>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let m = match msg {
        WsMessage::Text(t) => Message::Text(t.into()),
        WsMessage::Binary(b) => Message::Binary(b.into()),
        WsMessage::Ping(p) => Message::Ping(p.into()),
        WsMessage::Pong(p) => Message::Pong(p.into()),
        WsMessage::Close => Message::Close(None),
    };
    sink.send(m).await.map_err(|error| WsError::Transport {
        message: error.to_string(),
    })
}

#[derive(Debug, Error)]
pub enum WsError {
    #[error("websocket message serialization failed")]
    Serialize(#[source] serde_json::Error),
    #[error("websocket transport failed: {message}")]
    Transport { message: String },
}

pub(crate) type WsHandlerFn =
    Arc<dyn Fn(Request, WsSocket) -> BoxFuture<()> + Send + Sync + 'static>;

pub(crate) struct WsRouteEntry {
    pub pattern: PathPattern,
    pub handler: WsHandlerFn,
    pub name: Option<String>,
}

/// 把业务函数收成统一 WS handler。
pub trait IntoWsHandler<T>: Clone + Send + Sync + 'static {
    fn into_ws_handler(self) -> WsHandlerFn;
}

impl<F, Fut> IntoWsHandler<(WsSocket,)> for F
where
    F: Fn(WsSocket) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn into_ws_handler(self) -> WsHandlerFn {
        Arc::new(move |_req, socket| {
            let f = self.clone();
            Box::pin(async move { f(socket).await })
        })
    }
}

impl<F, Fut> IntoWsHandler<(Request, WsSocket)> for F
where
    F: Fn(Request, WsSocket) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    fn into_ws_handler(self) -> WsHandlerFn {
        Arc::new(move |req, socket| {
            let f = self.clone();
            Box::pin(async move { f(req, socket).await })
        })
    }
}

/// 是否为 WebSocket 升级请求。
pub fn is_upgrade_request(headers: &HeaderMap) -> bool {
    let upgrade = headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    let connection = headers
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(',')
                .any(|p| p.trim().eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let key = headers.get("sec-websocket-key").is_some();
    upgrade && connection && key
}

/// RFC6455 accept 值。
pub fn accept_key(sec_websocket_key: &str) -> String {
    const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let mut hasher = Sha1::new();
    hasher.update(sec_websocket_key.as_bytes());
    hasher.update(GUID.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

/// 构建 101 握手响应（body 为空）。
pub fn switching_protocols(sec_websocket_key: &str) -> HyperResponse<Body> {
    let accept = accept_key(sec_websocket_key);
    let mut res = HyperResponse::new(body_full(Bytes::new()));
    *res.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    res.headers_mut()
        .insert(header::UPGRADE, HeaderValue::from_static("websocket"));
    res.headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    if let Ok(v) = HeaderValue::from_str(&accept) {
        res.headers_mut()
            .insert(HeaderName::from_static("sec-websocket-accept"), v);
    }
    res.headers_mut().insert(
        HeaderName::from_static("sec-websocket-version"),
        HeaderValue::from_static("13"),
    );
    res
}

/// 升级连接并跑业务 handler。
pub async fn run_upgraded(
    hyper_req: HyperRequest<Incoming>,
    namix_req: Request,
    handler: WsHandlerFn,
) {
    match hyper::upgrade::on(hyper_req).await {
        Ok(upgraded) => {
            let io = TokioIo::new(upgraded);
            let stream = WebSocketStream::from_raw_socket(io, Role::Server, None).await;
            let socket = WsSocket { inner: stream };
            handler(namix_req, socket).await;
        }
        Err(err) => eprintln!("websocket upgrade failed: {err}"),
    }
}

/// 从 hyper 请求构造轻量 Namix Request（WS 无 body）。
pub fn namix_request_from_hyper(
    req: &HyperRequest<Incoming>,
    params: Vec<(String, String)>,
) -> Request {
    let mut namix = Request::new(
        req.method().clone(),
        req.uri().clone(),
        req.headers().clone(),
        Bytes::new(),
    );
    namix.set_params(params);
    namix
}

pub fn method_get() -> Method {
    Method::GET
}

/// 业务侧拒绝升级时的普通 HTTP 响应。
pub fn reject(status: StatusCode, msg: impl Into<Bytes>) -> Response {
    Response::new(status, crate::core::content_type::ContentType::Text, msg)
}
