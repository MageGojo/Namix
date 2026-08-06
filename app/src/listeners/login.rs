//! 登录成功后的副作用（鉴权 / 写 LoginLog 在控制器完成）。
//!
//! 启动时 `all()` 挂监听；控制器里 `dispatch(UserLoggedIn { .. })`。

use namix::prelude::*;

use crate::events::user_logged_in::UserLoggedIn;

/// 挂上本文件全部监听器。
pub fn all() {
    // 审计：谁在何时何地登录
    listen(|e: &UserLoggedIn| {
        namix::log::info!(
            "audit: user#{} ({}) logged in from {}",
            e.user_id,
            e.username,
            e.ip
        );
        Reply::ok(format!("audit · login {} @ {}", e.username, e.ip))
    });

    // 安全：本地环回视为受信，其它 IP 打一条「新环境」提醒（示意）
    listen(|e: &UserLoggedIn| {
        let trusted = e.ip == "127.0.0.1" || e.ip == "::1" || e.ip == "localhost";
        if trusted {
            Reply::ok(format!("security · trusted ip {}", e.ip))
        } else {
            namix::log::info!(
                "security: new login environment for {} from {}",
                e.username,
                e.ip
            );
            Reply::ok(format!("security · notify new env @ {}", e.ip))
        }
    });
}
