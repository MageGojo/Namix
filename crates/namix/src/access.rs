//! Simple role → permission map (Spatie-style, one convention, not four RBAC tables).
//!
//! ```ignore
//! namix::access::install(
//!     namix::access::Access::new()
//!         .role("admin", &["*"])
//!         .role("user", &["posts.create"]),
//! );
//! namix::access::allows("admin", "admin.access"); // true via *
//! ```

use std::collections::BTreeMap;
use std::sync::RwLock;

#[derive(Clone, Debug, Default)]
pub struct Access {
    roles: BTreeMap<String, Vec<String>>,
}

impl Access {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn role(mut self, role: impl Into<String>, permissions: &[&str]) -> Self {
        self.roles.insert(
            role.into(),
            permissions.iter().map(|p| (*p).to_string()).collect(),
        );
        self
    }

    pub fn allows(&self, role: &str, permission: &str) -> bool {
        let Some(perms) = self.roles.get(role) else {
            return false;
        };
        perms.iter().any(|item| item == "*" || item == permission)
    }

    pub fn permissions(&self, role: &str) -> &[String] {
        self.roles.get(role).map(Vec::as_slice).unwrap_or(&[])
    }
}

static ACCESS: RwLock<Option<Access>> = RwLock::new(None);

pub fn install(access: Access) {
    *ACCESS.write().expect("access lock") = Some(access);
}

pub fn current() -> Access {
    ACCESS
        .read()
        .expect("access lock")
        .clone()
        .unwrap_or_default()
}

pub fn allows(role: &str, permission: &str) -> bool {
    current().allows(role, permission)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_grants_every_permission() {
        let access = Access::new()
            .role("admin", &["*"])
            .role("user", &["posts.create"]);
        assert!(access.allows("admin", "admin.access"));
        assert!(access.allows("user", "posts.create"));
        assert!(!access.allows("user", "admin.access"));
        assert!(!access.allows("guest", "posts.create"));
    }
}
