//! 简易大厅聊天：进程内广播 + 按用户 id 的在线名单。
//!
//! 契约（`ChatUser` / `ChatMessage`）经 `ViewData` 生成到前端，
//! 身份以 `userId` 为准（类似 Laravel Echo 里比对 `Auth::id()`）。

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use namix::prelude::ViewData;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::services::session::LoginUser;

const CAPACITY: usize = 256;

/// 聊天室里的用户身份（页面 props / WS presence / hello 共用）。
#[derive(Debug, Clone, Serialize, Deserialize, ViewData, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatUser {
    pub id: u64,
    pub username: String,
}

impl From<&LoginUser> for ChatUser {
    fn from(u: &LoginUser) -> Self {
        Self {
            id: u.id,
            username: u.username.clone(),
        }
    }
}

/// 一条聊天消息（WS `type: "chat"` 的载荷字段）。
#[derive(Debug, Clone, Serialize, Deserialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub user_id: u64,
    pub username: String,
    pub text: String,
    pub at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMsg {
    #[serde(rename = "chat")]
    Chat { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMsg {
    /// 连上后立刻下发，确认本连接身份（类似 private channel auth）。
    #[serde(rename = "hello")]
    Hello { me: ChatUser },
    #[serde(rename = "chat")]
    Chat {
        #[serde(flatten)]
        message: ChatMessage,
    },
    #[serde(rename = "system")]
    System { text: String },
    #[serde(rename = "presence")]
    Presence { users: Vec<ChatUser> },
}

struct OnlineEntry {
    username: String,
    connections: usize,
}

struct HubInner {
    tx: broadcast::Sender<ServerMsg>,
    /// user_id → 连接计数（同账号多页互不踩）。
    online: Mutex<BTreeMap<u64, OnlineEntry>>,
}

fn hub() -> &'static HubInner {
    static HUB: OnceLock<HubInner> = OnceLock::new();
    HUB.get_or_init(|| {
        let (tx, _) = broadcast::channel(CAPACITY);
        HubInner {
            tx,
            online: Mutex::new(BTreeMap::new()),
        }
    })
}

#[derive(Clone, Default)]
pub struct ChatHub;

impl ChatHub {
    pub fn new() -> Self {
        Self
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerMsg> {
        hub().tx.subscribe()
    }

    pub fn publish(&self, msg: ServerMsg) {
        let _ = hub().tx.send(msg);
    }

    pub fn join(&self, user: &ChatUser) {
        let first_connection = {
            let mut online = hub().online.lock().expect("chat online");
            let entry = online.entry(user.id).or_insert_with(|| OnlineEntry {
                username: user.username.clone(),
                connections: 0,
            });
            entry.username = user.username.clone();
            entry.connections += 1;
            entry.connections == 1
        };
        if first_connection {
            self.publish(ServerMsg::System {
                text: format!("{} 加入了聊天室", user.username),
            });
            self.broadcast_presence();
        }
    }

    pub fn leave(&self, user: &ChatUser) {
        let last_connection = {
            let mut online = hub().online.lock().expect("chat online");
            let Some(entry) = online.get_mut(&user.id) else {
                return;
            };
            entry.connections = entry.connections.saturating_sub(1);
            if entry.connections == 0 {
                online.remove(&user.id);
                true
            } else {
                false
            }
        };
        if last_connection {
            self.publish(ServerMsg::System {
                text: format!("{} 离开了聊天室", user.username),
            });
            self.broadcast_presence();
        }
    }

    pub fn say(&self, user: &ChatUser, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let text = if text.chars().count() > 500 {
            let clipped: String = text.chars().take(500).collect();
            format!("{clipped}…")
        } else {
            text.to_string()
        };
        self.publish(ServerMsg::Chat {
            message: ChatMessage {
                user_id: user.id,
                username: user.username.clone(),
                text,
                at: now_secs(),
            },
        });
    }

    fn broadcast_presence(&self) {
        let users = {
            let online = hub().online.lock().expect("chat online");
            online
                .iter()
                .map(|(&id, e)| ChatUser {
                    id,
                    username: e.username.clone(),
                })
                .collect::<Vec<_>>()
        };
        self.publish(ServerMsg::Presence { users });
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{ChatHub, ChatUser, hub};

    #[test]
    fn presence_keeps_user_until_their_last_connection_leaves() {
        let user = ChatUser {
            id: 9_001,
            username: "__namix_chat_presence_test__".into(),
        };
        let chat = ChatHub::new();

        chat.join(&user);
        chat.join(&user);
        assert_eq!(
            hub()
                .online
                .lock()
                .expect("chat online")
                .get(&user.id)
                .map(|e| e.connections),
            Some(2)
        );

        chat.leave(&user);
        assert_eq!(
            hub()
                .online
                .lock()
                .expect("chat online")
                .get(&user.id)
                .map(|e| e.connections),
            Some(1)
        );

        chat.leave(&user);
        assert!(!hub().online.lock().expect("chat online").contains_key(&user.id));
    }
}
