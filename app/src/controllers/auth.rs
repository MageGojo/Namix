//! 注册 / 登录 / 登出。
//!
//! - GET 页面仍走命名路由（`/login`、`/register`）
//! - 写操作用 `#[server]` → TSX `callRust` / `server.login(...)`，**不必**再手写 POST 路由
//! - 失败返回 `ActionError`（`errors` 字段袋），前端 `useForm` 自动挂到对应 input

use namix::prelude::*;
use namix::server_fn;
use serde::Serialize;

use crate::events::user_logged_in::UserLoggedIn;
use crate::events::user_registered::UserRegistered;
use crate::models::user::User;
use crate::services::password_reset::PasswordResetService;
use crate::services::session::{
    session_id_from, SessionService, LEGACY_SESSION_COOKIE, SESSION_COOKIE,
};
use crate::services::user::UserService;
use crate::validators::login_form::LoginRequest;
use crate::validators::register_form::RegisterRequest;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct RegisterPage {
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct LoginPage {
    pub error: Option<String>,
    pub redirect: String,
    pub brand_icon: String,
    pub registered_count: u64,
}

// ── GET 页面 ─────────────────────────────────────────────────

pub async fn register(req: Request) -> Response {
    req.view("register")
        .island()
        .title("注册")
        .data(RegisterPage {
            error: req.flash().error,
        })
        .render()
}

pub async fn login(req: Request) -> Response {
    let registered_count = User::list().await.len() as u64;

    req.view("login")
        .island()
        .title("登录")
        .data(LoginPage {
            error: req.flash().error,
            redirect: req.query_or("redirect", "/me").to_string(),
            // 空 = 前端用打包的默认 logo（views/assets/namix.svg）；也可填外链/CDN
            brand_icon: String::new(),
            registered_count,
        })
        .render()
}

// ── Server Actions（≈ Leptos #[server]）──────────────────────
// 验证器：`validators/{register,login}_form.rs` → `FormRequest::from_values(&req)`

#[derive(Debug, Serialize)]
pub struct AuthOk {
    pub redirect: String,
}

#[derive(Debug, Serialize)]
pub struct PasswordResetOk {
    pub accepted: bool,
}

/// TSX: `import { register } from '../generated/actions/register'`
/// 规则见 `validators/register_form.rs`（Required / Between / Regex / Confirmed …）
#[server(name = "register", seal = ["password", "password_confirmation"])]
pub async fn register_action(req: Request) -> Result<ActionOk<AuthOk>, AppError> {
    let form = RegisterRequest::from_values(&req)?;
    let user = UserService::new()
        .register(&form.username, &form.password)
        .await?;

    let sessions = SessionService::new();
    let session_id = sessions.rotate(session_id_from(&req).as_deref(), &user);

    let outcome = dispatch(UserRegistered {
        user_id: user.id,
        username: user.username.clone(),
    });
    log_outcome("register", &outcome);

    Ok(ActionOk::new(AuthOk {
        redirect: "/me".into(),
    })
    .with_cookie_options(SESSION_COOKIE, session_id, SessionService::cookie_options())
    .with_clear_cookie(LEGACY_SESSION_COOKIE))
}

/// TSX: `import { login } from '../generated/actions/login'`
/// 规则见 `validators/login_form.rs`
#[server(name = "login", seal = ["password"])]
pub async fn login_action(req: Request) -> Result<ActionOk<AuthOk>, ActionError> {
    let form = LoginRequest::from_values(&req)?;

    let users = UserService::new();
    let Some(user) = users
        .authenticate(&form.username, &form.password)
        .await
    else {
        // 字段级错误：前端可 mapErrors 成中文，或直接展示
        return Err(ActionError::field(
            "password",
            "invalid username or password",
        ));
    };

    let _ = users.record_login(user.id, "127.0.0.1").await;
    let sessions = SessionService::new();
    let session_id = sessions.rotate(session_id_from(&req).as_deref(), &user);

    let outcome = dispatch(UserLoggedIn {
        user_id: user.id,
        username: user.username.clone(),
        ip: "127.0.0.1".into(),
    });
    log_outcome("login", &outcome);

    Ok(ActionOk::new(AuthOk {
        redirect: form.redirect,
    })
    .with_cookie_options(SESSION_COOKIE, session_id, SessionService::cookie_options())
    .with_clear_cookie(LEGACY_SESSION_COOKIE))
}

/// SSR 顶栏无 hydrate 时用：`GET /logout`
pub async fn logout_page(req: Request) -> Response {
    if let Some(sid) = session_id_from(&req) {
        SessionService::new().revoke(&sid);
    }
    Response::redirect_see_other("/")
        .with_clear_cookie_options(SESSION_COOKIE, SessionService::cookie_options())
        .with_clear_cookie(LEGACY_SESSION_COOKIE)
}

/// TSX：`import { logout } from '../generated/actions/logout'`
#[server(name = "logout")]
pub async fn logout_action(req: Request) -> Result<ActionOk<AuthOk>, ActionError> {
    if let Some(sid) = session_id_from(&req) {
        SessionService::new().revoke(&sid);
    }

    Ok(ActionOk::new(AuthOk {
        redirect: "/".into(),
    })
    .with_clear_cookie_options(SESSION_COOKIE, SessionService::cookie_options())
    .with_clear_cookie(LEGACY_SESSION_COOKIE))
}

/// 用户主动结束所有设备会话（当前设备也会被清除）。
#[server(name = "logout_all")]
pub async fn logout_all_action(req: Request) -> Result<ActionOk<AuthOk>, AppError> {
    let user = req
        .get::<crate::services::session::LoginUser>()
        .ok_or(AppError::Unauthenticated)?;
    SessionService::new().revoke_all_for_user(user.id);
    Ok(ActionOk::new(AuthOk {
        redirect: "/".into(),
    })
    .with_clear_cookie_options(SESSION_COOKIE, SessionService::cookie_options())
    .with_clear_cookie(LEGACY_SESSION_COOKIE))
}

/// Request a one-time reset token. The response remains identical for missing
/// accounts so an endpoint cannot be used for account enumeration.
#[server(name = "password_reset_request", seal = ["username"])]
pub async fn password_reset_request_action(
    req: Request,
) -> Result<ActionOk<PasswordResetOk>, AppError> {
    let input = server_fn::expand_input_map(&req).map_err(AppError::bad_request)?;
    let username = input
        .get("username")
        .map(String::as_str)
        .unwrap_or("")
        .trim();
    if username.is_empty() {
        return Err(AppError::validation("username", "username is required"));
    }

    if let Some(user) = User::find_by_username(username).await
        && let Some(profile) = user.load_profile().await
    {
        let email = profile.email.trim();
        if !email.is_empty() {
            let token = PasswordResetService.issue(user.id);
            let message =
                format!("Use this one-time password reset token within 30 minutes: {token}");
            if let Err(error) =
                Mail::send(MailMessage::new(email, "Reset your Namix password").text(message))
            {
                namix::log::warn!("password reset mail failed: {error}");
            }
        }
    }

    Ok(ActionOk::new(PasswordResetOk { accepted: true }))
}

/// Complete a reset token flow and revoke every previous device session.
#[server(name = "password_reset_confirm", seal = ["token", "password"])]
pub async fn password_reset_confirm_action(req: Request) -> Result<ActionOk<AuthOk>, AppError> {
    let input = server_fn::expand_input_map(&req).map_err(AppError::bad_request)?;
    let token = input.get("token").map(String::as_str).unwrap_or("").trim();
    let password = input.get("password").map(String::as_str).unwrap_or("");
    if token.is_empty() {
        return Err(AppError::validation("token", "reset token is required"));
    }
    if password.len() < 12 {
        return Err(AppError::validation(
            "password",
            "password must be at least 12 characters",
        ));
    }
    let user_id = PasswordResetService
        .consume(token)
        .ok_or_else(|| AppError::validation("token", "reset token is invalid or expired"))?;
    UserService::new()
        .reset_password(user_id, password)
        .await
        ?;
    SessionService::new().revoke_all_for_user(user_id);

    Ok(ActionOk::new(AuthOk {
        redirect: "/login".into(),
    })
    .with_clear_cookie_options(SESSION_COOKIE, SessionService::cookie_options())
    .with_clear_cookie(LEGACY_SESSION_COOKIE))
}

fn log_outcome(kind: &str, outcome: &Outcome) {
    let summary = outcome.summary();
    if summary.is_empty() {
        return;
    }
    if outcome.all_ok() {
        namix::log::info!("{kind} listeners: {summary}");
    } else {
        namix::log::warn!("{kind} listeners (partial fail): {summary}");
    }
}
