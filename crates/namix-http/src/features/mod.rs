//! 可选框架能力。由依赖方通过 Cargo features 打开：
//! `namix = { features = ["models"] }`
//!
//! 业务目录仍由各应用自己的 `namix.toml` + `namix-build` 同步。

#[cfg(feature = "models")]
pub mod models;

#[cfg(feature = "services")]
pub mod services;

#[cfg(feature = "requests")]
pub mod requests;

#[cfg(feature = "pages")]
pub mod pages;
