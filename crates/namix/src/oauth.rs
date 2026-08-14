//! Dev-oriented social login (Socialite-shaped). Real GitHub/Google adapters
//! register the same [`SocialProvider`] trait later; this crate ships a loopback
//! `dev` provider so the route/session path can be exercised without OAuth.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::AppError;
use crate::Request;

#[derive(Clone, Debug)]
pub struct SocialUser {
    pub provider: String,
    pub provider_id: String,
    pub username: String,
    pub email: Option<String>,
}

pub trait SocialProvider: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn authorize_url(&self, req: &Request, state: &str) -> String;
    fn user_from_callback(&self, req: &Request) -> Result<SocialUser, AppError>;
}

static PROVIDERS: RwLock<Option<HashMap<String, Arc<dyn SocialProvider>>>> = RwLock::new(None);

pub fn register(provider: impl SocialProvider) {
    let mut guard = PROVIDERS.write().expect("oauth provider lock");
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(provider.name().to_string(), Arc::new(provider));
}

pub fn provider(name: &str) -> Option<Arc<dyn SocialProvider>> {
    PROVIDERS.read().ok()?.as_ref()?.get(name).cloned()
}

/// Loopback provider: `/auth/dev` → `/auth/dev/callback?username=alice`.
pub struct DevProvider;

impl SocialProvider for DevProvider {
    fn name(&self) -> &'static str {
        "dev"
    }

    fn authorize_url(&self, req: &Request, _state: &str) -> String {
        let hint = req.query_or("username", "alice");
        format!("/auth/dev/callback?username={hint}")
    }

    fn user_from_callback(&self, req: &Request) -> Result<SocialUser, AppError> {
        let username = req.query("username").unwrap_or_else(|| "alice".into());
        if username.trim().is_empty() {
            return Err(AppError::validation("username", "username.required"));
        }
        let email = req
            .query("email")
            .unwrap_or_else(|| format!("{username}@oauth.namix.local"));
        Ok(SocialUser {
            provider: "dev".into(),
            provider_id: username.clone(),
            username,
            email: Some(email),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_dev_provider() {
        register(DevProvider);
        assert!(provider("dev").is_some());
    }
}
