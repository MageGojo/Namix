//! 用户领域服务：注册 / 鉴权 / 资料 / 发帖 —— **写**路径。
//!
//! 简单读库用模型 API（≈ Eloquent）：
//! `User::find` / `User::all` / `user.load_posts()` 等，不要再经本 Service 转发。

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use namix::{AppError, db::{self, DbResult}};
use sha2::{Digest, Sha256};

use crate::models::login_log::LoginLog;
use crate::models::post::Post;
use crate::models::profile::Profile;
use crate::models::user::User;

#[derive(Clone, Default)]
pub struct UserService;

impl UserService {
    pub fn new() -> Self {
        Self
    }

    /// 新密码使用随机盐的 Argon2id PHC 字符串。
    pub fn hash_password(password: &str) -> Result<String, AppError> {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|error| AppError::internal_message(format!("password hash failed: {error}")))
    }

    /// 验证 Argon2id 哈希，并兼容早期示例数据库的固定 SHA-256 格式。
    fn verify_password(password: &str, stored: &str) -> bool {
        if let Ok(hash) = PasswordHash::new(stored) {
            return Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok();
        }
        stored == Self::legacy_hash_password(password)
    }

    fn legacy_hash_password(password: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"namix-demo-salt:");
        hasher.update(password.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn needs_password_upgrade(stored: &str) -> bool {
        PasswordHash::new(stored)
            .map(|hash| hash.algorithm.as_str() != "argon2id")
            .unwrap_or(true)
    }

    /// 注册：User + 空 Profile（1:1）
    pub async fn register(&self, username: &str, password: &str) -> Result<User, AppError> {
        if username == "root" {
            return Err(AppError::validation("username", "username is reserved"));
        }
        if User::find_by_username(username).await.is_some() {
            return Err(AppError::conflict("username already taken"));
        }

        let username = username.to_string();
        let name = username.clone();
        let password_hash = Self::hash_password(password)?;

        db::run(move |mut db| {
            let username = username.clone();
            let name = name.clone();
            let password_hash = password_hash.clone();
            async move {
                let user = toasty::create!(User {
                    username: username.as_str(),
                    password_hash: password_hash.as_str(),
                    name: name.as_str(),
                    is_vip: false,
                    email_verified_at: None,
                })
                .exec(&mut db)
                .await?;

                toasty::create!(Profile {
                    user_id: user.id,
                    display_name: name.as_str(),
                    email: "",
                    bio: "",
                })
                .exec(&mut db)
                .await?;

                Ok(user)
            }
        })
        .await
        .map_err(AppError::internal)
    }

    /// 登录校验；早期 SHA-256 示例哈希会在成功登录后升级为 Argon2id。
    pub async fn authenticate(&self, username: &str, password: &str) -> Option<User> {
        let user = User::find_by_username(username).await?;
        if !Self::verify_password(password, &user.password_hash) {
            return None;
        }
        if !Self::needs_password_upgrade(&user.password_hash) {
            return Some(user);
        }

        let password_hash = match Self::hash_password(password) {
            Ok(value) => value,
            Err(error) => {
                namix::log::warn!("password hash upgrade skipped: {error}");
                return Some(user);
            }
        };
        let user_id = user.id;
        let upgraded = db::run(move |mut db| {
            let password_hash = password_hash.clone();
            async move {
                let mut current = User::get_by_id(&mut db, user_id).await?;
                toasty::update!(current {
                    password_hash: password_hash.as_str(),
                })
                .exec(&mut db)
                .await?;
                User::get_by_id(&mut db, user_id).await
            }
        })
        .await;
        match upgraded {
            Ok(user) => Some(user),
            Err(error) => {
                namix::log::warn!("password hash upgrade failed: {error}");
                Some(user)
            }
        }
    }

    /// Password-reset completion uses the same Argon2id write path.  Existing
    /// sessions are revoked by the controller after this succeeds.
    pub async fn reset_password(&self, user_id: u64, password: &str) -> Result<User, AppError> {
        let password_hash = Self::hash_password(password)?;
        db::run(move |mut db| {
            let password_hash = password_hash.clone();
            async move {
                let mut user = User::get_by_id(&mut db, user_id).await?;
                toasty::update!(user {
                    password_hash: password_hash.as_str(),
                })
                .exec(&mut db)
                .await?;
                User::get_by_id(&mut db, user_id).await
            }
        })
        .await
        .map_err(AppError::internal)
    }

    /// 单向关联：写登录日志。
    pub async fn record_login(&self, user_id: u64, ip: &str) -> DbResult<LoginLog> {
        let ip = ip.to_string();
        db::run(move |mut db| {
            let ip = ip.clone();
            async move {
                toasty::create!(LoginLog {
                    user_id,
                    ip: ip.as_str(),
                })
                .exec(&mut db)
                .await
            }
        })
        .await
    }

    /// 保存个人资料（没有则 create）。
    pub async fn save_profile(
        &self,
        user_id: u64,
        display_name: &str,
        email: &str,
        bio: &str,
    ) -> Result<Profile, AppError> {
        let display_name = display_name.to_string();
        let email = email.to_string();
        let bio = bio.to_string();

        db::run(move |mut db| {
            let display_name = display_name.clone();
            let email = email.clone();
            let bio = bio.clone();
            async move {
                let user = User::get_by_id(&mut db, user_id).await?;
                if let Some(mut profile) = user.profile().exec(&mut db).await? {
                    toasty::update!(profile {
                        display_name: display_name.as_str(),
                        email: email.as_str(),
                        bio: bio.as_str(),
                    })
                    .exec(&mut db)
                    .await?;
                    Ok(profile)
                } else {
                    toasty::create!(Profile {
                        user_id,
                        display_name: display_name.as_str(),
                        email: email.as_str(),
                        bio: bio.as_str(),
                    })
                    .exec(&mut db)
                    .await
                }
            }
        })
        .await
        .map_err(AppError::internal)
    }

    /// 发帖。
    pub async fn create_post(
        &self,
        user_id: u64,
        title: &str,
        body: &str,
    ) -> Result<Post, AppError> {
        let title = title.to_string();
        let body = body.to_string();
        db::run(move |mut db| {
            let title = title.clone();
            let body = body.clone();
            async move {
                toasty::create!(Post {
                    title: title.as_str(),
                    body: body.as_str(),
                    user_id,
                })
                .exec(&mut db)
                .await
            }
        })
        .await
        .map_err(AppError::internal)
    }

    /// 更新文章内容（调用方须已 `authorize`）。
    pub async fn update_post(
        &self,
        post_id: u64,
        title: &str,
        body: &str,
    ) -> Result<Post, AppError> {
        let title = title.to_string();
        let body = body.to_string();
        db::run(move |mut db| {
            let title = title.clone();
            let body = body.clone();
            async move {
                let mut post = Post::get_by_id(&mut db, post_id).await?;
                toasty::update!(post {
                    title: title.as_str(),
                    body: body.as_str(),
                })
                .exec(&mut db)
                .await?;
                Post::get_by_id(&mut db, post_id).await
            }
        })
        .await
        .map_err(AppError::internal)
    }

    /// 删除文章（调用方须已 `authorize`）。
    pub async fn delete_post(&self, post_id: u64) -> Result<(), AppError> {
        db::run(move |mut db| async move {
            Post::filter(Post::fields().id().eq(post_id))
                .delete()
                .exec(&mut db)
                .await
        })
        .await
        .map_err(AppError::internal)
    }

    /// 设置 VIP（种子 / 管理用）。
    pub async fn set_vip(&self, user_id: u64, is_vip: bool) -> Result<User, AppError> {
        db::run(move |mut db| async move {
            let mut user = User::get_by_id(&mut db, user_id).await?;
            toasty::update!(user { is_vip: is_vip }).exec(&mut db).await?;
            User::get_by_id(&mut db, user_id).await
        })
        .await
        .map_err(AppError::internal)
    }
}

#[cfg(test)]
mod tests {
    use super::UserService;

    #[test]
    fn argon2_hashes_are_salted_and_verifiable() {
        let first = UserService::hash_password("Secret1!").expect("hash password");
        let second = UserService::hash_password("Secret1!").expect("hash password");

        assert!(first.starts_with("$argon2id$"));
        assert_ne!(first, second);
        assert!(UserService::verify_password("Secret1!", &first));
        assert!(!UserService::verify_password("wrong", &first));
    }

    #[test]
    fn legacy_hashes_remain_verifiable_during_upgrade() {
        let legacy = UserService::legacy_hash_password("Secret1!");
        assert!(UserService::verify_password("Secret1!", &legacy));
        assert!(!UserService::verify_password("wrong", &legacy));
    }
}
