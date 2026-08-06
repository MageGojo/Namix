//! 关联示例种子：更新 Profile / 发 Post / Tag 多对多 / LoginLog

use namix::db::{self, DbResult};

use crate::models::post::Post;
use crate::models::post_tag::PostTag;
use crate::models::tag::Tag;
use crate::models::user::User;
use crate::services::user::UserService;

pub struct RelationsSeeder;

impl RelationsSeeder {
    pub async fn run() -> DbResult<()> {
        if !Post::list().await.is_empty() {
            namix::log::info!("relations already seeded");
            return Ok(());
        }

        let alice = User::find_by_username("alice")
            .await
            .ok_or_else(|| toasty::Error::from_args(format_args!("alice missing; seed users first")))?;

        // 注册时已建空 Profile；这里补全 1:1 字段
        UserService::new()
            .save_profile(alice.id, "Alice", "alice@example.com", "Alice's bio")
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;

        let post = UserService::new()
            .create_post(alice.id, "Hello Namix", "first post")
            .await
            .map_err(|e| toasty::Error::from_args(format_args!("{e}")))?;

        let rust = db::run(|mut db| async move {
            toasty::create!(Tag { name: "rust" }).exec(&mut db).await
        })
        .await?;
        let web = db::run(|mut db| async move {
            toasty::create!(Tag { name: "web" }).exec(&mut db).await
        })
        .await?;

        db::run({
            let post_id = post.id;
            let tag_id = rust.id;
            move |mut db| async move {
                toasty::create!(PostTag { post_id, tag_id })
                    .exec(&mut db)
                    .await
            }
        })
        .await?;
        db::run({
            let post_id = post.id;
            let tag_id = web.id;
            move |mut db| async move {
                toasty::create!(PostTag { post_id, tag_id })
                    .exec(&mut db)
                    .await
            }
        })
        .await?;

        UserService::new().record_login(alice.id, "127.0.0.1").await?;

        let alice_posts = alice.load_posts().await;
        let alice_profile = alice.load_profile().await;
        namix::log::info!(
            "alice posts={}, has_profile={}",
            alice_posts.len(),
            alice_profile.is_some()
        );

        let post_tags = post.load_tags().await;
        namix::log::info!(
            "post tags={:?}",
            post_tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>()
        );

        namix::log::info!("seeded relations: profile/post/tags/login_log for alice");
        Ok(())
    }
}
