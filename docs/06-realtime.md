# SSE 与 WebSocket / WSS

Namix 在 HTTP 核心里提供可读的实时通道 API。

| 协议 | 用法 | 传输 |
|------|------|------|
| **SSE** | `GET` + `text/event-stream` | 单向：服务端 → 浏览器 |
| **WS** | `Route::ws` + Upgrade | 双向：明文 `ws://` |
| **WSS** | 同一路径 | 走 HTTPS 监听即为 `wss://` |

---

## 设计要点

1. **SSE**：响应 body 改为可流式 `BoxBody`；业务用 `Sse::channel()` / `Sse::stream` 推 `SseEvent`。
2. **WS**：在读 body 之前匹配升级请求 → 101 + `hyper::upgrade` → `tokio-tungstenite`。
3. **WSS**：不另开协议栈；`Server` 开了 HTTPS 后，客户端连 `wss://host/path` 即可。
4. **路由**：SSE 仍是普通 `Route::get`；WS 用 `Route::ws`（catalog 里 method 记为 `WS`）。

---

## SSE

```rust
use namix::prelude::*;
use std::time::Duration;

pub async fn ticks(_req: Request) -> Response {
    let (sse, tx) = Sse::channel();
    tokio::spawn(async move {
        for n in 1..=5 {
            if tx
                .send(SseEvent::data(format!("{n}")).event("tick"))
                .await
                .is_err()
            {
                break; // 客户端断开
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    sse.into_response()
}

// 或闭包版
pub async fn ticks2(_req: Request) -> Response {
    Sse::stream(|tx| async move {
        let _ = tx.send(SseEvent::json(&serde_json::json!({ "ok": true }))).await;
    })
    .into_response()
}
```

路由：

```rust
Route::get("/sse/ticks", ticks).name("sse.ticks").register()
```

浏览器：

```js
const es = new EventSource('/sse/ticks')
es.addEventListener('tick', (e) => console.log(e.data))
es.onmessage = (e) => console.log('message', e.data)
```

`SseEvent` 常用链：`.data` / `.json` / `.event` / `.id` / `.retry`。
`SseSender::comment("ping")` 发 keep-alive 注释行。

---

## WebSocket / WSS

```rust
use namix::prelude::*;

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
```

路由：

```rust
Route::ws("/ws/echo", echo).name("ws.echo").register()
```

处理器也可以只收 socket：`async fn echo(mut socket: WsSocket)`。

浏览器：

```js
// HTTP
const ws = new WebSocket(`ws://${location.host}/ws/echo`)
// 若页面是 https://
const wss = new WebSocket(`wss://${location.host}/ws/echo`)

ws.onmessage = (e) => console.log(e.data)
ws.send('hello')
```

`WsSocket`：`recv` / `send` / `send_text` / `send_json` / `close`。

---

## 示例（本仓库）

| 名称 | 路径 |
|------|------|
| `sse.ticks` | `GET /sse/ticks` → `controllers/realtime.rs` |
| `ws.echo` | `WS /ws/echo` → 同上 |
| `chat` / `ws.chat` | `GET /chat` + `WS /ws/chat` → `controllers/chat.rs` |

```bash
nx dev -p 3000
# 另开终端
curl -N http://127.0.0.1:3000/sse/ticks
```

---

## 聊天室（鉴权 WS）

示例大厅：页面 Island，浏览器连命名路由 `ws.chat`；握手阶段用 **Cookie 会话**解析用户，未登录发 System 提示后关闭。

```rust
// controllers/chat.rs（节选）
pub async fn page(req: Request, user: AuthUser) -> Response {
    req.view("chat")
        .island()
        .title("聊天室")
        .data(ChatPage {
            title: "聊天室".into(),
            me: ChatUser::from(&*user),
        })
        .render()
}

pub async fn socket(req: Request, socket: WsSocket) {
    let login = /* session_id_from + SessionService::resolve */;
    // Hello { me } → subscribe → hub.join → 收 ClientMsg::Chat → hub.say
}
```

前端 hook（`views/namix.ts` 导出）：

```ts
import { useChatChannel } from '../namix'

const { me, status, users, lines, send } = useChatChannel(pageMe)
// 连接 route.ws.chat()；身份以服务端 hello.me 为准
send('hello') // → JSON { type: 'chat', text }
```

要点：

1. **不要**用用户名比对「是否自己」——用 `me.id`。
2. WS 不走 HTTP 中间件栈；鉴权在 handler 内读握手请求的 Cookie。
3. 广播与 presence 在 `services/chat.rs` 的 `ChatHub`。

---

## 注意

- SSE / WS 适合长连接；前面若有反向代理，需关闭缓冲（已设 `X-Accel-Buffering: no`），并调大超时。
- HTTP/3 路径仍缓冲完整 body，**实时通道请走 HTTP/1.1 或 HTTPS(H1)**。
- 鉴权可在 handler 里读 `req` 的 cookie / header（WS 握手阶段的 HTTP 头会带上）。