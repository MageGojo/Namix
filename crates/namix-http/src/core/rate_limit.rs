//! Small in-memory fixed-window rate limiter.
//!
//! It is intentionally backend-neutral: applications can use this limiter in
//! development and replace the same policy boundary with Redis later.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::error::AppError;
use super::middleware::{MiddlewareFn, wrap_middleware};
use super::request::{ClientIp, Request};

/// Keying strategy for a rate-limit policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RateLimitScope {
    Ip,
    /// Use `req.set_attr("namix.rate_limit.user", user_id)` when available;
    /// anonymous requests fall back to their client IP.
    UserOrIp,
}

/// A maximum request count over a rolling window.
#[derive(Clone, Debug)]
pub struct RateLimitPolicy {
    pub max_requests: usize,
    pub window: Duration,
    pub scope: RateLimitScope,
}

impl RateLimitPolicy {
    pub fn per_ip(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            scope: RateLimitScope::Ip,
        }
    }

    pub fn per_user_or_ip(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests,
            window,
            scope: RateLimitScope::UserOrIp,
        }
    }
}

#[derive(Default, Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume a slot.  `namespace` keeps independent budgets for login,
    /// registration, actions, uploads, and application-specific endpoints.
    pub fn check(
        &self,
        req: &Request,
        namespace: &str,
        policy: &RateLimitPolicy,
    ) -> Result<(), AppError> {
        if policy.max_requests == 0 || policy.window.is_zero() {
            return Ok(());
        }
        let now = Instant::now();
        let key = format!("{namespace}:{}", subject(req, policy.scope));
        let mut buckets = self.buckets.lock().expect("rate limit state");
        let bucket = buckets.entry(key).or_default();
        while bucket
            .front()
            .is_some_and(|at| now.duration_since(*at) >= policy.window)
        {
            bucket.pop_front();
        }
        if bucket.len() >= policy.max_requests {
            let retry_after = bucket
                .front()
                .map(|at| {
                    policy
                        .window
                        .saturating_sub(now.duration_since(*at))
                        .as_secs()
                        .max(1)
                })
                .unwrap_or(1);
            return Err(AppError::RateLimited { retry_after });
        }
        bucket.push_back(now);
        Ok(())
    }

    /// Build a regular middleware from the policy.
    pub fn middleware(self, namespace: impl Into<String>, policy: RateLimitPolicy) -> MiddlewareFn {
        let namespace = namespace.into();
        wrap_middleware(move |req, next| {
            let limiter = self.clone();
            let namespace = namespace.clone();
            let policy = policy.clone();
            async move {
                match limiter.check(&req, &namespace, &policy) {
                    Ok(()) => next.run(req).await,
                    Err(error) => error.into_response_for(&req),
                }
            }
        })
    }

    /// Apply the policy only to multipart requests and conventional upload
    /// endpoints.  It covers framework upload handlers without penalising
    /// unrelated POST forms.
    pub fn upload_middleware(self, policy: RateLimitPolicy) -> MiddlewareFn {
        wrap_middleware(move |req, next| {
            let limiter = self.clone();
            let policy = policy.clone();
            async move {
                let upload = req
                    .header("content-type")
                    .is_some_and(|value| value.contains("multipart/form-data"))
                    || req.path().contains("/upload");
                if upload && let Err(error) = limiter.check(&req, "upload", &policy) {
                    return error.into_response_for(&req);
                }
                next.run(req).await
            }
        })
    }
}

fn subject(req: &Request, scope: RateLimitScope) -> String {
    if scope == RateLimitScope::UserOrIp
        && let Some(user) = req.attr("namix.rate_limit.user")
        && !user.is_empty()
    {
        return format!("user:{user}");
    }
    let ip = req
        .get::<ClientIp>()
        .map(|ClientIp(ip)| *ip)
        .unwrap_or(IpAddr::from([0, 0, 0, 0]));
    format!("ip:{ip}")
}

/// Explicit opt-in point for middleware which resolves a logged-in user.
pub fn set_user_subject(req: &mut Request, user_id: impl ToString) {
    req.set_attr("namix.rate_limit.user", user_id.to_string());
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::{HeaderMap, Method, Uri};

    use super::*;

    fn req() -> Request {
        let mut req = Request::new(
            Method::POST,
            Uri::from_static("/x"),
            HeaderMap::new(),
            Bytes::new(),
        );
        req.set(ClientIp(IpAddr::from([127, 0, 0, 1])));
        req
    }

    #[test]
    fn rejects_after_budget_is_exhausted() {
        let limiter = RateLimiter::new();
        let policy = RateLimitPolicy::per_ip(2, Duration::from_secs(60));
        assert!(limiter.check(&req(), "login", &policy).is_ok());
        assert!(limiter.check(&req(), "login", &policy).is_ok());
        assert!(matches!(
            limiter.check(&req(), "login", &policy),
            Err(AppError::RateLimited { .. })
        ));
    }

    #[test]
    fn user_scope_is_independent_from_anonymous_ip_scope() {
        let limiter = RateLimiter::new();
        let policy = RateLimitPolicy::per_user_or_ip(1, Duration::from_secs(60));
        let mut first = req();
        set_user_subject(&mut first, 7);
        let mut second = req();
        set_user_subject(&mut second, 8);
        assert!(limiter.check(&first, "action", &policy).is_ok());
        assert!(limiter.check(&second, "action", &policy).is_ok());
    }
}
