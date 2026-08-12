//! 首页 / VIP 厅。
//!
//! 授权只在服务端分支：下发给页面的是**已定稿的展示数据**（问候语、导航链接），
//! 绝不包含 `userId` / `isVip` / roles / token。

use namix::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::middleware::session::current;
use crate::models::user::User;
use crate::services::session::LoginUser;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct NavLink {
    pub label: String,
    pub href: String,
}

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct HomePage {
    pub title: String,
    /// 纯展示文案（由服务端按身份写好），不是授权字段。
    pub greeting: String,
    pub users_count: u64,
    /// 服务端已按身份筛好的导航；前端只负责渲染，不知「为什么」有这些链接。
    pub nav_links: Vec<NavLink>,
}

pub async fn index(req: Request) -> Response {
    let users_count = User::list().await.len() as u64;
    let auth = AuthView::new(current(&req));
    let (greeting, nav_links) = auth.choose(
        || {
            (
                "未登录".to_string(),
                guest_nav(),
            )
        },
        |user| {
            (
                format!("你好，{}", user.username),
                user_nav(user),
            )
        },
    );

    req.view("home")
        .ssr()
        .title("User App")
        .data(HomePage {
            title: "User App".into(),
            greeting,
            users_count,
            nav_links,
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

fn guest_nav() -> Vec<NavLink> {
    vec![
        link("Home", "/"),
        link("Login", "/login"),
        link("Register", "/register"),
        link("Demo", "/demo"),
    ]
}

fn user_nav(user: &LoginUser) -> Vec<NavLink> {
    let mut links = vec![
        link("Home", "/"),
        link("Me", "/me"),
        link("Posts", "/posts"),
        link("Chat", "/chat"),
        link("Public", "/profile/1"),
        link("Demo", "/demo"),
    ];
    // VIP 链接只在服务端插入；前端看不到 isVip 字段。
    if user.is_vip {
        links.push(link("VIP", "/vip"));
    }
    links
}

fn link(label: &str, href: &str) -> NavLink {
    NavLink {
        label: label.into(),
        href: href.into(),
    }
}
