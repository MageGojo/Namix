//! 登录日志 → 表 `login_logs`
//!
//! **单向关联**：只有这里 `belongs_to` User。
//! User 上故意不写 `has_many`，演示「不必双向」（Laravel 也可只写一边）。

use super::user::User;

#[derive(Clone, Debug, toasty::Model)]
#[table = "login_logs"]
pub struct LoginLog {
    #[key]
    #[auto]
    pub id: u64,

    #[index]
    pub user_id: u64,

    pub ip: String,

    /// 登录时刻（append-only，无 updated_at）
    #[auto]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key = user_id, references = id)]
    pub user: toasty::Deferred<User>,
}
