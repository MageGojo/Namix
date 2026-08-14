//! Namix 单应用业务包（扁平 MVC）。
//!
//! - `models` / `services` / `seeders` — 数据层（碰库只在 services）
//! - `controllers` / `routes` / `middleware` — HTTP 层
//! - `validators` / `events` / `listeners` — 表单与副作用

include!("namix_modules.rs");
pub mod facades;
pub mod prelude;
pub mod route;
pub mod view;
