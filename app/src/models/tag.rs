//! 标签 → 表 `tags`（与 Post 多对多）

use super::post::Post;
use super::post_tag::PostTag;

#[derive(Clone, Debug, toasty::Model)]
#[table = "tags"]
pub struct Tag {
    #[key]
    #[auto]
    pub id: u64,

    #[unique]
    pub name: String,

    #[auto]
    pub created_at: jiff::Timestamp,

    #[auto]
    pub updated_at: jiff::Timestamp,

    #[has_many(pair = tag)]
    pub post_tags: toasty::Deferred<Vec<PostTag>>,

    /// 多对多只读：Tag → Post
    #[has_many(via = post_tags.post)]
    pub posts: toasty::Deferred<Vec<Post>>,
}
