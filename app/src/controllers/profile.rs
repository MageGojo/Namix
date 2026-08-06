//! 公开资料页。

use namix::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::models::user::User;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePage {
    pub title: String,
    pub display_name: String,
    pub username: String,
    pub email: String,
    pub bio: String,
    pub viewer: String,
    pub post_titles: Vec<String>,
}

pub async fn show(req: Request, Path(id): Path<u64>, user: AuthUser) -> Response {
    let Some(db_user) = User::find(id).await else {
        return not_found();
    };
    let profile = db_user.load_profile().await;
    let posts = db_user.load_posts().await;
    let (display_name, email, bio) = match &profile {
        Some(p) => (p.display_name.clone(), p.email.clone(), p.bio.clone()),
        None => (db_user.name.clone(), String::new(), String::new()),
    };

    let post_titles: Vec<String> = posts.into_iter().map(|p| p.title).collect();

    req.view("profile")
        .ssr()
        .title(&display_name)
        .data(ProfilePage {
            title: display_name.clone(),
            display_name,
            username: db_user.username,
            email,
            bio,
            viewer: user.username.clone(),
            post_titles,
        })
        .render()
}
