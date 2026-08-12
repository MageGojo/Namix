//! 门禁中间件（只做「过 / 不过」）。
//!
//! - 会话解析见 [`super::session::hydrate`]
//! - 提取器见 [`super::extract::AuthUser`]
//!
//! ```ignore
//! Route::get("/me", me::show)
//!     .middleware(require_login)
//!     .register();
//!
//! Route::get("/login", auth::login)
//!     .middleware(require_guest)
//!     .register();
//! ```

use namix::prelude::*;

use crate::route;
use crate::services::session::{session_id_from, LoginUser, SessionService};

/// 必须已登录；否则跳转登录页。
///
/// 若全局未挂 `hydrate`，这里会自行 resolve 一次并注入 [`LoginUser`]。
pub async fn require_login(mut req: Request, next: Next) -> Response {
    if req.get::<LoginUser>().is_none() {
        let Some(id) = session_id_from(&req) else {
            return req.redirect_guest_to(route::main::login);
        };
        match SessionService::new().resolve(&id) {
            Ok(Some(user)) => {
                namix::set_user_subject(&mut req, user.id);
                req.set(user);
            }
            Ok(None) => return req.redirect_guest_to(route::main::login),
            Err(error) => return error.into_response_for(&req),
        };
    }
    next.run(req).await
}

/// 仅访客：已登录则离开登录/注册页（默认回首页，可用 `?redirect=`）。
pub async fn require_guest(mut req: Request, next: Next) -> Response {
    if req.get::<LoginUser>().is_none() && let Some(token) = session_id_from(&req) {
        match SessionService::new().resolve(&token) {
            Ok(Some(user)) => {
                namix::set_user_subject(&mut req, user.id);
                req.set(user);
            }
            Ok(None) => {}
            Err(error) => return error.into_response_for(&req),
        }
    }

    if req.get::<LoginUser>().is_some() {
        let target = req
            .query("redirect")
            .filter(|value| is_local_path(value))
            .unwrap_or_else(|| "/".into());
        return Response::redirect_see_other(target);
    }

    next.run(req).await
}

/// 必须为 VIP（依赖上游已注入 [`LoginUser`]，请先挂 `require_login`）。
pub async fn require_vip(req: Request, next: Next) -> Response {
    let Some(user) = req.get::<LoginUser>() else {
        return req.redirect_guest_to(route::main::login);
    };

    if !user.is_vip {
        return Response::new(
            namix::http::StatusCode::FORBIDDEN,
            ContentType::Text,
            "VIP only — ask an admin to grant is_vip",
        );
    }

    next.run(req).await
}
