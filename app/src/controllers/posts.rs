//! 我的文章。

use namix::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::models::user::User;
use crate::route;
use crate::services::user::UserService;
use crate::validators::post_form::PostRequest;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct PostItem {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct PostsPage {
    pub title: String,
    pub username: String,
    pub error: Option<String>,
    pub items: Vec<PostItem>,
}

pub async fn index(req: Request, user: AuthUser) -> Response {
    let Some(db_user) = User::find(user.id).await else {
        return req.redirect_guest_to(route::main::login);
    };
    let posts = db_user.load_posts().await;
    let flash = req.flash();

    let items: Vec<PostItem> = posts
        .into_iter()
        .map(|p| PostItem {
            title: p.title,
            body: p.body,
        })
        .collect();

    req.view("posts")
        .ssr()
        .title("我的文章")
        .data(PostsPage {
            title: "我的文章".into(),
            username: user.username.clone(),
            error: flash.error,
            items,
        })
        .render()
}

pub async fn create(req: Request, user: AuthUser, form: PostRequest) -> Response {
    match UserService::new()
        .create_post(user.id, &form.title, &form.body)
        .await
    {
        Ok(_) => req.see_other_to(route::main::posts),
        Err(error) => req.redirect_error_to(route::main::posts, error.message()),
    }
}
