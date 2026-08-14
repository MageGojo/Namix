//! 邮箱验证结果页。

use crate::prelude::*;
use serde::Serialize;

use crate::middleware::extract::AuthUser;
use crate::services::email_verification::EmailVerificationService;
use crate::services::session::session_id_from;
use crate::models::user::User;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct EmailVerifyPage {
    pub ok: bool,
    pub message: String,
}

pub async fn show(req: Request) -> Response {
    let token = req.query_or("token", "");
    if token.trim().is_empty() {
        return req
            .view(Page::EmailVerify)
            .ssr()
            .title("验证邮箱")
            .data(EmailVerifyPage {
                ok: false,
                message: "缺少验证令牌。".into(),
            })
            .render();
    }

    match EmailVerificationService.verify(&token).await {
        Ok(user_id) => {
            if let Some(sid) = session_id_from(&req)
                && let Ok(Some(mut record)) = namix::session::current().get(&sid)
                && record.user_id == user_id
            {
                record.email_verified = true;
                let _ = namix::session::current().put(&sid, &record);
            }
            req.view(Page::EmailVerify)
                .ssr()
                .title("验证邮箱")
                .data(EmailVerifyPage {
                    ok: true,
                    message: "邮箱已验证，可以继续使用。".into(),
                })
                .render()
        }
        Err(_) => req
            .view(Page::EmailVerify)
            .ssr()
            .title("验证邮箱")
            .data(EmailVerifyPage {
                ok: false,
                message: "验证链接无效或已过期。".into(),
            })
            .render(),
    }
}

pub async fn resend(req: Request, user: AuthUser) -> Response {
    let Some(db_user) = User::find(user.id).await else {
        return req.redirect_guest_to(AppRoute::Login);
    };
    if db_user.email_verified_at.is_some() {
        return req.redirect_ok_to(AppRoute::Me);
    }
    let email = db_user
        .load_profile()
        .await
        .map(|p| p.email)
        .unwrap_or_default();
    match EmailVerificationService.notify(user.id, &email) {
        Ok(()) => req.redirect_ok_to(AppRoute::Me),
        Err(error) => req.redirect_error_to(AppRoute::Me, error.message()),
    }
}
