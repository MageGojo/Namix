//! 首页 / VIP 厅。
//!
//! 授权只在服务端分支：下发给页面的是**已定稿的展示数据**（问候语、导航链接），
//! 绝不包含 `userId` / `isVip` / roles / token。

use crate::prelude::*;
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

    req.view(Page::Home)
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
        link("Home", AppRoute::Home.href()),
        link("Login", AppRoute::Login.href()),
        link("Register", AppRoute::Register.href()),
        link("Demo", AppRoute::Demo.href()),
    ]
}

fn user_nav(user: &LoginUser) -> Vec<NavLink> {
    let mut links = vec![
        link("Home", AppRoute::Home.href()),
        link("Me", AppRoute::Me.href()),
        link("Posts", AppRoute::Posts.href()),
        link("Chat", AppRoute::Chat.href()),
        link(
            "Public",
            AppRoute::Profile
                .to(&[("id", "1")])
                .unwrap_or_else(|| "/profile/1".into()),
        ),
        link("Demo", AppRoute::Demo.href()),
    ];
    if user.is_vip {
        links.push(link("VIP", AppRoute::Vip.href()));
    }
    if user.role == "admin" {
        links.push(link("Admin", AppRoute::AdminUsers.href()));
    }
    links
}

fn link(label: &str, href: impl Into<String>) -> NavLink {
    NavLink {
        label: label.into(),
        href: href.into(),
    }
}
