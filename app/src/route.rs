//! 类型化路由名（自动生成，勿手改）。
//!
//! - 注册：`routes/web.rs` 里 `name: "login"`（或链式 `.name("login")`；均可省略）
//! - 使用：`route::main::login` / `req.redirect_to(route::main::me)`
//! - 前端：`route.login()`（由运行时 catalog 生成 `views/routes.ts`）
//! - 调试：`GET /__namix/routes`

include!(concat!(env!("OUT_DIR"), "/namix_route_names.rs"));
