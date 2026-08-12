//! 会话水合：把 Cookie/Bearer 里的 `session_id` 解析为 [`LoginUser`]。
//!
//! 全局挂载后，控制器用 [`current`] / [`AuthUser`]，不必自己查库。

use namix::prelude::*;

use crate::services::session::{session_id_from, LoginUser, SessionService};

/// 全局中间件：有合法会话则 `req.set(LoginUser)`，无会话则放行（公开页）。
pub async fn hydrate(mut req: Request, next: Next) -> Response {
    if req.get::<LoginUser>().is_none() && let Some(id) = session_id_from(&req) {
        match SessionService::new().resolve(&id) {
            Ok(Some(user)) => {
                namix::set_user_subject(&mut req, user.id);
                req.set(user);
            }
            Ok(None) => {}
            Err(error) => return error.into_response_for(&req),
        }
    }
    next.run(req).await
}

/// 当前请求上的登录用户（需已跑过 [`hydrate`] 或 `require_login`）。
pub fn current(req: &Request) -> Option<&LoginUser> {
    req.get::<LoginUser>()
}
