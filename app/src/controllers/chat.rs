//! 登录用户大厅聊天（WebSocket）。

use namix::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::services::chat::{ChatHub, ChatUser, ClientMsg, ServerMsg};
use crate::services::session::{session_id_from, SessionService};

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct ChatPage {
    pub title: String,
    /// 当前登录用户（用 `me.id` 识别自己的消息，不要比用户名）。
    pub me: ChatUser,
}

/// GET /chat — Island 页，浏览器连命名路由 `ws.chat`。
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

/// WS /ws/chat — Cookie 会话鉴权后进入广播大厅。
pub async fn socket(req: Request, socket: WsSocket) {
    let Some(login) = resolve_user(&req) else {
        let mut socket = socket;
        let _ = socket
            .send_json(&ServerMsg::System {
                text: "请先登录".into(),
            })
            .await;
        let _ = socket.close().await;
        return;
    };

    let me = ChatUser::from(&login);
    let hub = ChatHub::new();
    let (mut writer, mut reader) = socket.split();

    // 先确认身份（前端以 hello.me 为准），再订阅，最后 join——
    // 这样本连接也能收到自己进房触发的 presence。
    if writer
        .send_json(&ServerMsg::Hello { me: me.clone() })
        .await
        .is_err()
    {
        return;
    }

    let mut rx = hub.subscribe();
    let push = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if writer.send_json(&msg).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    hub.join(&me);

    while let Some(msg) = reader.recv().await {
        match msg {
            WsMessage::Close => break,
            WsMessage::Ping(_) => {}
            WsMessage::Text(text) => match serde_json::from_str::<ClientMsg>(&text) {
                Ok(ClientMsg::Chat { text }) => hub.say(&me, &text),
                Err(_) => hub.say(&me, &text),
            },
            WsMessage::Binary(_) | WsMessage::Pong(_) => {}
        }
    }

    hub.leave(&me);
    push.abort();
}

fn resolve_user(req: &Request) -> Option<crate::services::session::LoginUser> {
    let id = session_id_from(req)?;
    SessionService::new().resolve(&id)
}
