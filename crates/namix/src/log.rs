//! 彩色日志（tracing + ANSI）。
//!
//! ```ignore
//! namix::log::init(); // Boot 已自动调用
//! namix::log::info!("ready");
//! namix::log::warn!(user = %name, "slow query");
//! ```
//!
//! 过滤：`RUST_LOG=debug` / `RUST_LOG=namix=info,app=debug`

use std::sync::Once;

pub use tracing::{Level, debug, error, info, trace, warn};

/// 初始化彩色终端日志（只生效一次）。
pub fn init() {
    static START: Once = Once::new();
    START.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .with_level(true)
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            .compact()
            .init();
    });
}

/// HTTP 状态码着色（绿/蓝/橙/红）。
pub fn color_status(code: u16) -> String {
    let (r, g, b) = match code {
        200..=299 => (61u8, 220, 151),
        300..=399 => (108, 182, 255),
        400..=499 => (255, 184, 108),
        _ => (255, 107, 107),
    };
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        format!("\x1b[1;38;2;{r};{g};{b}m{code}\x1b[0m")
    } else {
        code.to_string()
    }
}

/// 方法名轻微着色。
pub fn color_method(method: &str) -> String {
    let (r, g, b) = match method {
        "GET" => (108u8, 182, 255),
        "POST" => (61, 220, 151),
        "PUT" | "PATCH" => (255, 214, 102),
        "DELETE" => (255, 107, 107),
        _ => (200, 200, 200),
    };
    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        format!("\x1b[38;2;{r};{g};{b}m{method}\x1b[0m")
    } else {
        method.to_string()
    }
}
