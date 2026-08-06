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
            .register("alice", "Secret1!")
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;
        // alice = VIP 示范账号（bob 普通用户）
        svc.set_vip(alice.id, true)
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;
        svc.register("bob", "Secret1!")
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;
        namix::log::info!("seeded users: alice (vip), bob (password Secret1!)");
        Ok(())
    }
}
