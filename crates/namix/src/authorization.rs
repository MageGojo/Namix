//! Policy / Gate authorization primitives.

use crate::AppError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ability {
    ViewAny,
    View,
    Create,
    Update,
    Delete,
}

/// A model policy. Policies are ordinary Rust values, making them easy to unit
/// test and usable from SSR controllers, APIs, and Actions alike.
pub trait Policy<Actor, Resource> {
    fn allows(&self, actor: &Actor, ability: Ability, resource: Option<&Resource>) -> bool;
}

#[derive(Clone, Debug)]
pub struct Gate<Actor> {
    actor: Actor,
}

impl<Actor> Gate<Actor> {
    pub fn for_user(actor: Actor) -> Self {
        Self { actor }
    }
    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    pub fn allows<P, Resource>(
        &self,
        policy: &P,
        ability: Ability,
        resource: Option<&Resource>,
    ) -> bool
    where
        P: Policy<Actor, Resource>,
    {
        policy.allows(&self.actor, ability, resource)
    }

    pub fn authorize<P, Resource>(
        &self,
        policy: &P,
        ability: Ability,
        resource: Option<&Resource>,
    ) -> Result<(), AppError>
    where
        P: Policy<Actor, Resource>,
    {
        self.allows(policy, ability, resource)
            .then_some(())
            .ok_or(AppError::Forbidden)
    }
}

/// Concise standalone form for a one-off check.
pub fn authorize<Actor, P, Resource>(
    actor: &Actor,
    policy: &P,
    ability: Ability,
    resource: Option<&Resource>,
) -> Result<(), AppError>
where
    P: Policy<Actor, Resource>,
{
    policy
        .allows(actor, ability, resource)
        .then_some(())
        .ok_or(AppError::Forbidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone)]
    struct User {
        id: u64,
    }
    struct Post {
        owner_id: u64,
    }
    struct Posts;
    impl Policy<User, Post> for Posts {
        fn allows(&self, user: &User, ability: Ability, resource: Option<&Post>) -> bool {
            ability == Ability::Update && resource.is_some_and(|post| post.owner_id == user.id)
        }
    }
    #[test]
    fn gates_ownership() {
        assert!(
            authorize(
                &User { id: 1 },
                &Posts,
                Ability::Update,
                Some(&Post { owner_id: 1 })
            )
            .is_ok()
        );
        assert!(
            authorize(
                &User { id: 2 },
                &Posts,
                Ability::Update,
                Some(&Post { owner_id: 1 })
            )
            .is_err()
        );
    }
}
