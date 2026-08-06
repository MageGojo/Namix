//! 文章 → 表 `posts`
//!
//! - 双向 1:N：`belongs_to` User（pair = author）
//! - 多对多：via PostTag → Tag（≈ Laravel `belongsToMany`）

use namix::db;

use super::post_tag::PostTag;
use super::tag::Tag;
use super::user::User;

#[derive(Clone, Debug, toasty::Model)]
#[table = "posts"]
pub struct Post {
    #[key]
    #[auto]
    pub id: u64,

    pub title: String,
    pub body: String,

    #[index]
    pub user_id: u64,

    #[auto]
    pub created_at: jiff::Timestamp,

    #[auto]
    pub updated_at: jiff::Timestamp,

    /// 双向：指向作者
    #[belongs_to(key = user_id, references = id)]
    pub author: toasty::Deferred<User>,

    /// 中间表行（写多对多时改这个）
    #[has_many(pair = post)]
    pub post_tags: toasty::Deferred<Vec<PostTag>>,

    /// 多对多只读遍历：Post → Tag
    #[has_many(via = post_tags.tag)]
    pub tags: toasty::Deferred<Vec<Tag>>,
}

impl Post {
    /// ≈ `Post::find($id)`
    pub async fn find(id: u64) -> Option<Self> {
        db::optional(move |mut db| async move { Post::get_by_id(&mut db, id).await }).await
    }

    /// ≈ Laravel `Post::all()`（Toasty 占用 `all()` 查询构建器）
    pub async fn list() -> Vec<Self> {
        db::vec(|mut db| async move { Post::all().exec(&mut db).await }).await
    }

    /// ≈ `$post->tags`
    pub async fn load_tags(&self) -> Vec<Tag> {
        let post = self.clone();
        db::vec(move |mut db| async move { post.tags().exec(&mut db).await }).await
    }
}
