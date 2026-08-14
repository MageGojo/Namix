//! 框架常驻核心：任何业务都依赖这些能力，不进 feature 开关。

pub mod content_type;
pub mod controller;
pub mod crypt;
pub mod csrf;
pub mod error;
pub mod error_pages;
pub mod extract;
pub mod handler;
pub mod middleware;
pub mod proxy;
pub mod rate_limit;
pub mod request;
pub mod response;
pub mod routing;
pub mod server;
pub mod server_fn;
pub mod sse;
pub mod test_client;
pub mod transfer;
pub mod upload;
pub mod validate;
pub mod ws;
