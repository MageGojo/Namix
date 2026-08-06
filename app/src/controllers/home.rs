//! 首页 / VIP 厅。

use namix::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::middleware::session::current;
use crate::models::user::User;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct HomePage {
    pub title: String,
    pub username: Option<String>,
    pub is_vip: bool,
    pub users_count: u64,
}

pub async fn index(req: Request) -> Response {
    let login = current(&req);
    let users_count = User::list().await.len() as u64;

    req.view("home")
        .ssr()
        .title("User App")
        .data(HomePage {
            title: "User App".into(),
            username: login.map(|u| u.username.clone()),
            is_vip: login.map(|u| u.is_vip).unwrap_or(false),
            users_count,
        })
        .render()
}

/// VIP 专区（路由：`require_login` + `require_vip`）。
pub async fn vip_lounge(_req: Request, user: AuthUser) -> Response {
    text(format!(
        "welcome {}, this is the VIP lounge",
        user.username
    ))
}
