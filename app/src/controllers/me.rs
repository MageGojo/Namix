//! 个人资料。

use namix::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::models::user::User;
use crate::route;
use crate::services::user::UserService;
use crate::validators::profile_form::ProfileRequest;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct MePage {
    pub title: String,
    pub username: String,
    pub user_id: u64,
    pub display_name: String,
    pub email: String,
    pub bio: String,
    pub error: Option<String>,
    pub saved: bool,
}

pub async fn show(req: Request, user: AuthUser) -> Response {
    let Some(db_user) = User::find(user.id).await else {
        return req.redirect_guest_to(route::main::login);
    };
    let profile = db_user.load_profile().await;
    let flash = req.flash();

    let (display_name, email, bio) = match &profile {
        Some(p) => (p.display_name.clone(), p.email.clone(), p.bio.clone()),
        None => (db_user.name.clone(), String::new(), String::new()),
    };

    req.view("me")
        .ssr()
        .title("个人资料")
        .data(MePage {
            title: "个人资料".into(),
            username: db_user.username,
            user_id: db_user.id,
            display_name,
            email,
            bio,
            error: flash.error,
            saved: flash.success,
        })
        .render()
}

pub async fn save(req: Request, user: AuthUser, form: ProfileRequest) -> Response {
    match UserService::new()
        .save_profile(user.id, &form.display_name, &form.email, &form.bio)
        .await
    {
        Ok(_) => req.redirect_ok_to(route::main::me),
        Err(error) => req.redirect_error_to(route::main::me, error.message()),
    }
}
