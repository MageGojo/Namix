//! 业务侧一键导入：框架 prelude + 本应用的 `AppRoute` / `Page`。
//!
//! `Route` 仍是注册路由用的 `Route::get`；命名路由枚举叫 [`AppRoute`]，避免撞名。

pub use crate::route::{self, AppRoute};
pub use crate::view::{self, Page};
pub use namix::prelude::*;

pub use crate::middleware::session::{RequestAuth as _, current};
