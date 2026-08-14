//! 个人资料。

use crate::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::models::user::User;
use crate::services::user::UserService;
use crate::validators::profile_form::ProfileRequest;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct MePage {
    pub title: String,
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub bio: String,
    pub email_verified: bool,
    pub avatar_url: Option<String>,
    pub error: Option<String>,
    pub saved: bool,
    pub csrf_token: String,
}

pub async fn show(req: Request, user: AuthUser) -> Response {
    let Some(db_user) = User::find(user.id).await else {
        return req.redirect_guest_to(AppRoute::Login);
    };
    let profile = db_user.load_profile().await;
    let flash = req.flash();

    let (display_name, email, bio, avatar_url) = match &profile {
        Some(p) => (
            p.display_name.clone(),
            p.email.clone(),
            p.bio.clone(),
            if p.avatar_path.is_empty() {
                None
            } else {
                Some("/me/avatar".into())
            },
        ),
        None => (db_user.name.clone(), String::new(), String::new(), None),
    };

    req.view(Page::Me)
        .ssr()
        .title("个人资料")
        .data(MePage {
            title: "个人资料".into(),
            username: db_user.username,
            display_name,
            email,
            bio,
            email_verified: db_user.email_verified_at.is_some(),
            avatar_url,
            error: flash.error,
            saved: flash.success,
            csrf_token: req.csrf_token().to_string(),
        })
        .render()
}

pub async fn save(req: Request, user: AuthUser, form: ProfileRequest) -> Response {
    let users = UserService::new();
    if let Err(error) = users
        .save_profile(user.id, &form.display_name, &form.email, &form.bio)
        .await
    {
        return req.redirect_error_to(AppRoute::Me, error.message());
    }
    if let Some(avatar) = form.avatar.filter(|file| !file.is_empty()) {
        let ext = avatar.extension().to_ascii_lowercase();
        let ext = if ext.is_empty() { "png".into() } else { ext };
        let key = format!("avatars/{}.{}", user.id, ext);
        let storage = namix::Storage::new(namix::LocalStorage::new("./storage/app", "/me/avatar"));
        let policy = UploadPolicy {
            max_bytes: 2_000_000,
            allowed_extensions: vec!["png".into(), "jpg".into(), "jpeg".into(), "webp".into(), "gif".into()],
        };
        if let Err(error) = storage.put_with_policy(&key, &avatar.data, &policy) {
            return req.redirect_error_to(AppRoute::Me, error.to_string());
        }
        if let Err(error) = users.save_avatar(user.id, &key).await {
            return req.redirect_error_to(AppRoute::Me, error.message());
        }
    }
    req.redirect_ok_to(AppRoute::Me)
}

pub async fn avatar(req: Request, user: AuthUser) -> Response {
    let Some(db_user) = User::find(user.id).await else {
        return req.not_found();
    };
    let Some(profile) = db_user.load_profile().await else {
        return req.not_found();
    };
    if profile.avatar_path.is_empty() {
        return req.not_found();
    }
    let storage = namix::Storage::new(namix::LocalStorage::new("./storage/app", "/me/avatar"));
    match storage.get(&profile.avatar_path) {
        Ok(Some(bytes)) => {
            let ct = ContentType::from_path(&profile.avatar_path);
            Response::new(namix::http::StatusCode::OK, ct, bytes)
        }
        _ => req.not_found(),
    }
}
