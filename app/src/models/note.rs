//! 便签 → 表 `notes`（User 1:N，补齐厨房水槽里的最小业务模型）

use super::user::User;

#[derive(Clone, Debug, toasty::Model)]
#[table = "notes"]
pub struct Note {
    #[key]
    #[auto]
    pub id: u64,

    pub title: String,

    #[index]
    pub user_id: u64,

    #[auto]
    pub created_at: jiff::Timestamp,

    #[auto]
    pub updated_at: jiff::Timestamp,

    #[belongs_to(key = user_id, references = id)]
    pub author: toasty::Deferred<User>,
}
