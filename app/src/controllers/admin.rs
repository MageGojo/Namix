//! 后台用户表（权限演示 + DataTable）。

use crate::prelude::*;
use namix::{Paginator, QueryOptions, SortWhitelist};
use serde::Serialize;

use crate::models::user::User;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserRow {
    pub id: String,
    pub username: String,
    pub role: String,
    pub vip: String,
    pub verified: String,
}

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct AdminUsersPage {
    pub title: String,
    pub rows: Vec<AdminUserRow>,
    pub total: usize,
    pub per_page: usize,
    pub current_page: usize,
    pub last_page: usize,
    pub from: usize,
    pub to: usize,
}

pub async fn users(req: Request) -> Response {
    let query = QueryOptions::from_request(
        &req,
        &SortWhitelist::new(["id", "username"]),
        std::iter::empty::<&str>(),
    )
        .unwrap_or_else(|_| QueryOptions::default());
    let all = User::list().await;
    let page = Paginator::from_items(all, &query);
    let rows = page
        .data
        .iter()
        .map(|user| AdminUserRow {
            id: user.id.to_string(),
            username: user.username.clone(),
            role: user.role.clone(),
            vip: if user.is_vip { "yes".into() } else { "no".into() },
            verified: if user.email_verified_at.is_some() {
                "yes".into()
            } else {
                "no".into()
            },
        })
        .collect();

    req.view(Page::AdminUsers)
        .ssr()
        .title("用户")
        .data(AdminUsersPage {
            title: "用户".into(),
            rows,
            total: page.total,
            per_page: page.per_page,
            current_page: page.current_page,
            last_page: page.last_page,
            from: page.from,
            to: page.to,
        })
        .render()
}
