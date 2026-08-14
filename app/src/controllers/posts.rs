//! 我的文章。写路径经 `PostPolicy`：会话身份对照库记录 `user_id`。

use crate::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::models::post::Post;
use crate::models::user::User;
use crate::policies::post_policy::PostPolicy;
use crate::services::user::UserService;
use crate::validators::post_form::PostRequest;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct PostItem {
    pub id: u64,
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
    pub csrf_token: String,
}

pub async fn index(req: Request, user: AuthUser) -> Response {
    let Some(db_user) = User::find(user.id).await else {
        return req.redirect_guest_to(AppRoute::Login);
    };
    let posts = db_user.load_posts().await;
    let flash = req.flash();

    let items: Vec<PostItem> = posts
        .into_iter()
        .map(|p| PostItem {
            id: p.id,
            title: p.title,
            body: p.body,
        })
        .collect();

    req.view(Page::Posts)
        .ssr()
        .title("我的文章")
        .data(PostsPage {
            title: "我的文章".into(),
            username: user.username.clone(),
            error: flash.error,
            items,
            csrf_token: req.csrf_token().to_string(),
        })
        .render()
}

pub async fn create(req: Request, user: AuthUser, form: PostRequest) -> Result<Response, AppError> {
    authorize(&*user, &PostPolicy, Ability::Create, None)?;
    match UserService::new()
        .create_post(user.id, &form.title, &form.body)
        .await
    {
        Ok(_) => Ok(req.see_other_to(AppRoute::Posts)),
        Err(error) => Ok(req.redirect_error_to(AppRoute::Posts, error.message())),
    }
}

/// POST /posts/:id — 表单只带 title/body；归属以库中 `Post` 为准。
pub async fn update(
    req: Request,
    user: AuthUser,
    Path(id): Path<u64>,
    form: PostRequest,
) -> Result<Response, AppError> {
    let post = Post::find(id).await.or_not_found()?;
    authorize(&*user, &PostPolicy, Ability::Update, Some(&post))?;
    UserService::new()
        .update_post(post.id, &form.title, &form.body)
        .await?;
    Ok(req.see_other_to(AppRoute::Posts))
}

/// POST /posts/:id/delete — HTML 表单用 POST；仍走 Policy::Delete。
pub async fn destroy(req: Request, user: AuthUser, Path(id): Path<u64>) -> Result<Response, AppError> {
    let post = Post::find(id).await.or_not_found()?;
    authorize(&*user, &PostPolicy, Ability::Delete, Some(&post))?;
    UserService::new().delete_post(post.id).await?;
    Ok(req.see_other_to(AppRoute::Posts))
}
