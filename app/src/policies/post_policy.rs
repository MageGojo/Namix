//! 文章所有权策略：会话 `LoginUser` vs 库里的 `Post.user_id`。

use namix::prelude::*;

use crate::models::post::Post;
use crate::services::session::LoginUser;

pub struct PostPolicy;

impl Policy<LoginUser, Post> for PostPolicy {
    fn allows(&self, actor: &LoginUser, ability: Ability, resource: Option<&Post>) -> bool {
        match ability {
            Ability::Create => true,
            Ability::ViewAny => true,
            Ability::View | Ability::Update | Ability::Delete => {
                resource.is_some_and(|post| post.user_id == actor.id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor(id: u64) -> LoginUser {
        LoginUser {
            id,
            username: "alice".into(),
            is_vip: false,
            session_id: "sid".into(),
        }
    }

    fn post(owner: u64) -> Post {
        Post {
            id: 1,
            title: "t".into(),
            body: "b".into(),
            user_id: owner,
            created_at: jiff::Timestamp::UNIX_EPOCH,
            updated_at: jiff::Timestamp::UNIX_EPOCH,
            author: Default::default(),
            post_tags: Default::default(),
            tags: Default::default(),
        }
    }

    #[test]
    fn owner_can_update_and_delete() {
        let user = actor(7);
        let mine = post(7);
        assert!(authorize(&user, &PostPolicy, Ability::Update, Some(&mine)).is_ok());
        assert!(authorize(&user, &PostPolicy, Ability::Delete, Some(&mine)).is_ok());
    }

    #[test]
    fn stranger_is_forbidden() {
        let user = actor(7);
        let theirs = post(9);
        assert!(authorize(&user, &PostPolicy, Ability::Update, Some(&theirs)).is_err());
        assert!(authorize(&user, &PostPolicy, Ability::Delete, Some(&theirs)).is_err());
    }
}
