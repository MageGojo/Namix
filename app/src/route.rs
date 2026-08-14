//! 类型化路由名（自动生成，勿手改）。
//!
//! - 注册：`routes/web.rs` 里 `name: "login"`
//! - 使用：`AppRoute::Login`（枚举）或 `route::main::login`（同一值的别名）
//! - 拼 URL：`AppRoute::Profile.to(&[("id", "1")])`
//! - 前端：`route.login()` / `route(AppRoute.Login)`
//! - 调试：`GET /__namix/routes`

include!(concat!(env!("OUT_DIR"), "/namix_route_names.rs"));
