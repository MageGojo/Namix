//! 资料 → 表 `profiles`（User 双向 1:1，≈ Laravel `hasOne` / `belongsTo`）

use super::user::User;

#[derive(Clone, Debug, toasty::Model)]
#[table = "profiles"]
pub struct Profile {
    #[key]
    #[auto]
    pub id: u64,

    #[unique]
    pub user_id: u64,

    pub display_name: String,
    pub email: String,
    pub bio: String,
    pub avatar_path: String,

    #[auto]
    pub created_at: jiff::Timestamp,

    #[auto]
    pub updated_at: jiff::Timestamp,

    #[belongs_to(key = user_id, references = id)]
    pub user: toasty::Deferred<User>,
}
