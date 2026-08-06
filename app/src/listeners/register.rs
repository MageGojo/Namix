//! 注册成功后的副作用（落库已在 `UserService::register` 完成）。
//!
//! 启动时 `all()` 挂监听；控制器里 `dispatch(UserRegistered { .. })`。

use namix::prelude::*;

use crate::events::user_registered::UserRegistered;

/// 挂上本文件全部监听器。
pub fn all() {
    // 欢迎邮件（Mail 门面；当前 log/file 驱动会落 outbox）
    listen(|e: &UserRegistered| {
        let to = format!("{}@users.namix.local", e.username);
        match Mail::send(
            MailMessage::new(to, "欢迎加入 Namix")
                .text(format!("你好 {}，注册成功（user#{}）。", e.username, e.user_id)),
        ) {
            Ok(()) => Reply::ok(format!("welcome mail → {}", e.username)),
            Err(err) => {
                namix::log::error!("welcome mail failed: {err}");
                Reply::err(format!("welcome mail failed: {err}"))
            }
        }
    });

    // 审计日志
    listen(|e: &UserRegistered| {
        namix::log::info!("audit: user registered #{} ({})", e.user_id, e.username);
        Reply::ok(format!("audit · registered {}", e.username))
    });

    // 初始化默认资料占位（示意）
    listen(|e: &UserRegistered| {
        namix::log::info!("profile: seed defaults for #{}", e.user_id);
        Reply::ok(format!("profile seed · #{}", e.user_id))
    });
}
