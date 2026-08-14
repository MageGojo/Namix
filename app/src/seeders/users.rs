//! 用户种子数据（通过 UserService 写入，不直接拼 SQL）。

use namix::db::DbResult;

use crate::models::user::User;
use crate::services::user::UserService;

pub struct UsersSeeder;

impl UsersSeeder {
    pub async fn run() -> DbResult<()> {
        if !User::list().await.is_empty() {
            namix::log::info!("users already seeded");
            return Ok(());
        }

        let svc = UserService::new();
        let alice = svc
            .register("alice", "Secret1!", "alice@namix.local")
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;
        svc.set_vip(alice.id, true)
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;
        svc.set_role(alice.id, "admin")
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;
        svc.mark_email_verified(alice.id)
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;
        svc.register("bob", "Secret1!", "bob@namix.local")
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;
        namix::log::info!("seeded users: alice (admin/vip), bob (password Secret1!)");
        Ok(())
    }
}
