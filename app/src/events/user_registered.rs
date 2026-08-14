//! 用户注册成功 —— 控制器 `dispatch`，各功能 `listen`。

#[derive(Clone, Debug)]
pub struct UserRegistered {
    pub user_id: u64,
    pub username: String,
    pub email: String,
}
