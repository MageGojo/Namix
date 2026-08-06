//! SSE / WebSocket 演示。
//!
//! | 路径 | 协议 |
//! |------|------|
//! | `GET /sse/ticks` | Server-Sent Events |
//! | `WS  /ws/echo`   | WebSocket（HTTPS 下即 WSS） |

use std::time::Duration;

use namix::prelude::*;
use serde::Serialize;

#[derive(Serialize)]
struct Tick {
    n: u32,
    message: String,
}

/// SSE：每秒推一条 JSON，共 5 次后结束。
///
/// 浏览器：`new EventSource('/sse/ticks')`
pub async fn ticks(_req: Request) -> Response {
    let (sse, tx) = Sse::channel();
    tokio::spawn(async move {
        for n in 1..=5u32 {
            let ev = SseEvent::json(&Tick {
                n,
                message: format!("tick {n}"),
            })
            .event("tick")
            .id(n.to_string());
            if tx.send(ev).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let _ = tx
            .send(SseEvent::data("done").event("close"))
            .await;
    });
    sse.into_response()
}

/// WebSocket echo：原样回传文本/二进制。
///
/// 浏览器：`new WebSocket('ws://127.0.0.1:3000/ws/echo')`
/// HTTPS：`wss://…/ws/echo`
pub async fn echo(_req: Request, mut socket: WsSocket) {
    while let Some(msg) = socket.recv().await {
        match msg {
            WsMessage::Close => break,
            WsMessage::Ping(p) => {
                let _ = socket.send(WsMessage::Pong(p)).await;
            }
            other => {
                if socket.send(other).await.is_err() {
                    break;
                }
            }
        }
    }
}
