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
//! Route::get("/vip", home::vip_lounge)
//!     .middleware(require_login)
//!     .middleware(require_vip)
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
        let Some(user) = SessionService::new().resolve(&id) else {
            return req.redirect_guest_to(route::main::login);
        };
        req.set(user);
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
