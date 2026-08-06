//! 中间表 → `post_tags`（≈ Laravel pivot）
//!
//! - 唯一可写的多对多侧
//! - `#[unique(post_id, tag_id)]` ≈ pivot 防重复挂同一标签

use super::post::Post;
use super::tag::Tag;

#[derive(Clone, Debug, toasty::Model)]
#[table = "post_tags"]
#[unique(post_id, tag_id)]
pub struct PostTag {
    #[key]
    #[auto]
    pub id: u64,

    #[index]
    pub post_id: u64,

    #[index]
    pub tag_id: u64,

    /// pivot timestamps（Laravel `$table->timestamps()` on pivot）
    #[auto]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key = post_id, references = id)]
    pub post: toasty::Deferred<Post>,

    #[belongs_to(key = tag_id, references = id)]
    pub tag: toasty::Deferred<Tag>,
}
