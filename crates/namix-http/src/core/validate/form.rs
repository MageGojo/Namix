//! Form Request：校验通过后以结构体进入控制器；失败自动 redirect + flash。

use super::{ValidationError, Validator};
use crate::core::extract::FromRequest;
use crate::core::request::Request;
use crate::core::response::Response;
use crate::core::routing::NamedRoute;

/// 校验失败时的跳转目标。
#[derive(Debug, Clone, Copy)]
pub enum FormRedirect {
    /// `redirect` 查询参数 / Referer / `/`
    Back,
    /// 命名路由（与 `.name("login")` 一致）
    Named(&'static str),
}

impl FormRedirect {
    /// `FormRedirect::named(AppRoute::Login)`，与 `.name("login")` 同一字符串。
    pub fn named(route: impl NamedRoute) -> Self {
        Self::Named(route.route_name())
    }
}

/// Laravel Form Request 风格：进控制器即合法表单。
///
/// ```ignore
/// pub async fn register_submit(req: Request, form: RegisterRequest) -> Response {
///     users.register(&form.username, &form.password).await
/// }
/// ```
pub trait FormRequest: Sized + Send {
    /// 校验失败跳哪；默认回上一页。
    fn redirect_to() -> FormRedirect {
        FormRedirect::Back
    }

    /// 跑规则并组装结构体（不要在这里 redirect）。
    fn from_values(req: &Request) -> Result<Self, ValidationError>;
}

impl<T: FormRequest> FromRequest for T {
    fn from_request(req: &Request) -> Result<Self, Response> {
        match T::from_values(req) {
            Ok(form) => Ok(form),
            Err(err) => Err(match T::redirect_to() {
                FormRedirect::Back => err.redirect_back(req),
                FormRedirect::Named(name) => err.redirect_route(req, name),
            }),
        }
    }
}

/// 给 `FormRequest` 实现用的小入口。
pub fn validator(req: &Request) -> Validator<'_> {
    Validator::from_request(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct LoginRoute;

    impl NamedRoute for LoginRoute {
        fn route_name(self) -> &'static str {
            "login"
        }
    }

    #[test]
    fn named_accepts_typed_route() {
        assert!(matches!(
            FormRedirect::named(LoginRoute),
            FormRedirect::Named("login")
        ));
    }
}
