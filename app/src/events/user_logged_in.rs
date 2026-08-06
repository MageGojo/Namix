//! 用户登录成功 —— 控制器 `dispatch`，各功能 `listen`。

#[derive(Clone, Debug)]
pub struct UserLoggedIn {
    pub user_id: u64,
    pub username: String,
    /// 客户端 IP（示意：审计 / 异地登录提醒）。
    pub ip: String,
}
