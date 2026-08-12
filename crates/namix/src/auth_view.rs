//! Server-only auth branching for views.
//!
//! Controllers decide **what HTML/content** to ship. Never put roles, user ids,
//! VIP flags, or tokens into Island/SPA props — pass already-resolved
//! presentational data (nav links, greetings, sections).

use crate::authorization::{Ability, Policy};

/// Request-scoped actor view for guest / authenticated branching.
#[derive(Clone, Copy, Debug)]
pub struct AuthView<'a, A> {
    actor: Option<&'a A>,
}

impl<'a, A> AuthView<'a, A> {
    pub fn new(actor: Option<&'a A>) -> Self {
        Self { actor }
    }

    pub fn is_guest(self) -> bool {
        self.actor.is_none()
    }

    pub fn is_authenticated(self) -> bool {
        self.actor.is_some()
    }

    pub fn actor(self) -> Option<&'a A> {
        self.actor
    }

    /// Pick guest vs authenticated presentational payloads (no role flags).
    pub fn choose<T>(self, guest: impl FnOnce() -> T, authenticated: impl FnOnce(&A) -> T) -> T {
        match self.actor {
            Some(actor) => authenticated(actor),
            None => guest(),
        }
    }

    pub fn map_guest<T>(self, f: impl FnOnce() -> T) -> Option<T> {
        self.actor.is_none().then(f)
    }

    pub fn map_auth<T>(self, f: impl FnOnce(&A) -> T) -> Option<T> {
        self.actor.map(f)
    }

    /// Include a section only when a policy allows it — still return
    /// presentational `T`, never an `is_admin` flag for the client.
    pub fn when_allows<P, Resource, T>(
        self,
        policy: &P,
        ability: Ability,
        resource: Option<&Resource>,
        build: impl FnOnce(&A) -> T,
    ) -> Option<T>
    where
        P: Policy<A, Resource>,
    {
        let actor = self.actor?;
        policy
            .allows(actor, ability, resource)
            .then(|| build(actor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::Ability;

    struct Actor {
        admin: bool,
    }
    struct AdminPolicy;
    impl Policy<Actor, ()> for AdminPolicy {
        fn allows(&self, actor: &Actor, ability: Ability, _: Option<&()>) -> bool {
            ability == Ability::View && actor.admin
        }
    }

    #[test]
    fn choose_and_policy_sections_stay_server_side() {
        let guest = AuthView::new(None::<&Actor>);
        assert_eq!(guest.choose(|| "guest", |_| "user"), "guest");

        let admin = Actor { admin: true };
        let view = AuthView::new(Some(&admin));
        let section = view.when_allows(&AdminPolicy, Ability::View, None, |_| "admin-panel");
        assert_eq!(section, Some("admin-panel"));

        let user = Actor { admin: false };
        let view = AuthView::new(Some(&user));
        assert!(
            view.when_allows(&AdminPolicy, Ability::View, None, |_| "admin-panel")
                .is_none()
        );
    }
}
