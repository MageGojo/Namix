//! 请求提取器：从已水合的上下文取 [`LoginUser`]。

use crate::prelude::*;

use crate::services::session::LoginUser;

/// 已登录用户提取器（需路由挂了 `require_login`，或全局 `hydrate` 且确有会话）。
///
/// ```ignore
/// pub async fn me(req: Request, user: AuthUser) -> Response {
///     text(format!("hi {}", user.username))
/// }
/// ```
#[derive(Clone, Debug)]
pub struct AuthUser(pub LoginUser);

impl std::ops::Deref for AuthUser {
    type Target = LoginUser;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromRequest for AuthUser {
    fn from_request(req: &Request) -> Result<Self, Response> {
        req.get::<LoginUser>()
            .cloned()
            .map(AuthUser)
            .ok_or_else(|| req.redirect_guest_to(AppRoute::Login))
    }
}
