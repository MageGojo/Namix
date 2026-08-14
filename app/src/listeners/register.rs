//! 注册成功后的副作用（落库已在 `UserService::register` 完成）。
//!
//! 启动时 `all()` 挂监听；控制器里 `dispatch(UserRegistered { .. })`。

use namix::prelude::*;

use crate::events::user_registered::UserRegistered;
use crate::services::email_verification::EmailVerificationService;

/// 挂上本文件全部监听器。
pub fn all() {
    listen(|e: &UserRegistered| {
        match EmailVerificationService.notify(e.user_id, &e.email) {
            Ok(()) => Reply::ok(format!("verify mail → {}", e.email)),
            Err(err) => {
                namix::log::error!("verify mail failed: {err}");
                Reply::err(format!("verify mail failed: {err}"))
            }
        }
    });

    listen(|e: &UserRegistered| {
        let to = if e.email.trim().is_empty() {
            format!("{}@users.namix.local", e.username)
        } else {
            e.email.clone()
        };
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

    listen(|e: &UserRegistered| {
        namix::log::info!("audit: user registered #{} ({})", e.user_id, e.username);
        Reply::ok(format!("audit · registered {}", e.username))
    });
}
