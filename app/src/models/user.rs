//! 用户 → 表 `users`（≈ Laravel `User` Eloquent）
//!
//! | Laravel | Namix / Toasty |
//! |---|---|
//! | `$table = 'users'` | `#[table = "users"]` |
//! | `$timestamps` | `created_at` / `updated_at` + `#[auto]` |
//! | `hasOne(Profile)` | `#[has_one] profile` |
//! | `hasMany(Post)` | `#[has_many] posts` |
//! | `hasMany(Note)` | `#[has_many] notes` |
//! | `$hidden = ['password']` | `password_hash` + `#[serde(skip_serializing)]` |
//!
//! 简洁取数（≈ Eloquent；底层 Toasty + 全局 Db）：
//! ```ignore
//! User::find(1).await
//! User::find_by_username("alice").await
//! User::list().await          // ≈ Laravel User::all()（Toasty 占用了 all() 查询构建器）
//! user.load_profile().await
//! user.load_posts().await
//! ```

use namix::db;

use super::note::Note;
use super::post::Post;
use super::profile::Profile;

#[derive(Clone, Debug, toasty::Model)]
#[table = "users"]
pub struct User {
    #[key]
    #[auto]
    pub id: u64,

    #[unique]
    pub username: String,

    /// 密码哈希（≈ Laravel `$hidden`：API/视图层勿直接吐出此字段）。
    pub password_hash: String,

    pub name: String,

    /// VIP 角色（门禁用）。
    pub is_vip: bool,

    /// 邮箱验证时间（≈ `email_verified_at`）；未验证为 `None`。
    pub email_verified_at: Option<jiff::Timestamp>,

    /// Laravel `$timestamps`
    #[auto]
    pub created_at: jiff::Timestamp,

    #[auto]
    pub updated_at: jiff::Timestamp,

    #[has_many(pair = author)]
    pub posts: toasty::Deferred<Vec<Post>>,

    #[has_many(pair = author)]
    pub notes: toasty::Deferred<Vec<Note>>,

    #[has_one(pair = user)]
    pub profile: toasty::Deferred<Option<Profile>>,
}

impl User {
    /// ≈ `User::find($id)`
    pub async fn find(id: u64) -> Option<Self> {
        db::optional(move |mut db| async move { User::get_by_id(&mut db, id).await }).await
    }

    /// ≈ `User::where('username', $u)->first()`
    pub async fn find_by_username(username: impl Into<String>) -> Option<Self> {
        let username = username.into();
        db::optional(move |mut db| {
            let username = username.clone();
            async move { User::get_by_username(&mut db, username.as_str()).await }
        })
        .await
    }

    /// ≈ Laravel `User::all()`（Toasty 的 `User::all()` 是查询构建器，故命名 `list`）
    pub async fn list() -> Vec<Self> {
        db::vec(|mut db| async move { User::all().exec(&mut db).await }).await
    }

    /// ≈ `$user->profile`（hasOne）
    pub async fn load_profile(&self) -> Option<Profile> {
        let user = self.clone();
        db::run(move |mut db| async move { user.profile().exec(&mut db).await })
            .await
            .ok()
            .flatten()
    }

    /// ≈ `$user->posts`
    pub async fn load_posts(&self) -> Vec<Post> {
        let user = self.clone();
        db::vec(move |mut db| async move { user.posts().exec(&mut db).await }).await
    }

    /// ≈ `$user->notes`
    pub async fn load_notes(&self) -> Vec<Note> {
        let user = self.clone();
        db::vec(move |mut db| async move { user.notes().exec(&mut db).await }).await
    }
}
