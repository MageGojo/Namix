//! 会话：签名 Cookie → 过期、轮换和全设备撤销。
//!
//! - Cookie / Bearer 只携带不透明、签名的会话 token，不包含用户名
//! - 进程内 Store 是默认开发实现；接口边界可替换成 Redis / 数据库
//! - 登录会轮换当前会话；密码重置和“退出所有设备”按用户撤销全部会话

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use namix::CookieOptions;
use rand::RngCore;
use sha2::Sha256;

use crate::models::user::User;

/// 浏览器会话 Cookie（值 = 签名 token，不是用户名）。
pub const SESSION_COOKIE: &str = "namix_session";
/// 旧版误用 username 的 Cookie，登录/登出时顺带清掉。
pub const LEGACY_SESSION_COOKIE: &str = "namix_user";
/// 默认绝对会话时长；可由未来持久化驱动沿用同一语义。
pub const SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 7);

/// 请求上下文中的已登录用户（由 `middleware::session::hydrate` 注入）。
#[derive(Clone, Debug)]
pub struct LoginUser {
    pub id: u64,
    pub username: String,
    pub is_vip: bool,
    /// 内部原始 session id；不应回显到页面或 API。
    pub session_id: String,
}

#[derive(Clone, Debug)]
struct SessionRecord {
    user_id: u64,
    username: String,
    is_vip: bool,
    expires_at: SystemTime,
}

type Store = RwLock<HashMap<String, SessionRecord>>;

fn store() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(HashMap::new()))
}

#[derive(Clone, Default)]
pub struct SessionService;

impl SessionService {
    pub fn new() -> Self {
        Self
    }

    /// 签发有绝对过期时间的全新会话。返回值可直接写入 Cookie。
    pub fn issue(&self, user: &User) -> String {
        self.issue_for(user, SESSION_TTL)
    }

    pub fn issue_for(&self, user: &User, ttl: Duration) -> String {
        let id = new_session_id();
        let expires_at = SystemTime::now().checked_add(ttl).unwrap_or(SystemTime::now());
        store().write().expect("session store").insert(
            id.clone(),
            SessionRecord {
                user_id: user.id,
                username: user.username.clone(),
                is_vip: user.is_vip,
                expires_at,
            },
        );
        sign_token(&id)
    }

    /// 登录/重新认证时轮换 token，旧会话立即失效。
    pub fn rotate(&self, previous_token: Option<&str>, user: &User) -> String {
        if let Some(previous_token) = previous_token {
            self.revoke(previous_token);
        }
        self.issue(user)
    }

    /// 登出：销毁一个会话。无效签名不会触及 Store。
    pub fn revoke(&self, token: &str) {
        if let Some(id) = verified_id(token) {
            store().write().expect("session store").remove(&id);
        }
    }

    /// 当前密码重置、主动安全退出等场景使用：撤销该用户的所有设备。
    pub fn revoke_all_for_user(&self, user_id: u64) -> usize {
        let mut guard = store().write().expect("session store");
        let before = guard.len();
        guard.retain(|_, record| record.user_id != user_id);
        before - guard.len()
    }

    /// `session token` → [`LoginUser`]。过期记录会被顺手清理。
    pub fn resolve(&self, token: &str) -> Option<LoginUser> {
        let id = verified_id(token)?;
        let mut guard = store().write().expect("session store");
        let record = guard.get(&id)?.clone();
        if SystemTime::now() >= record.expires_at {
            guard.remove(&id);
            return None;
        }
        Some(LoginUser {
            id: record.user_id,
            username: record.username,
            is_vip: record.is_vip,
            session_id: id,
        })
    }

    pub fn cookie_options() -> CookieOptions {
        CookieOptions {
            secure: session_cookie_secure(),
            max_age: Some(SESSION_TTL.as_secs()),
            ..CookieOptions::default()
        }
    }
}

/// 从 Cookie 或 `Authorization: Bearer <signed-session-token>` 取 token。
pub fn session_id_from(req: &namix::Request) -> Option<String> {
    req.bearer()
        .map(str::to_string)
        .or_else(|| req.cookie(SESSION_COOKIE).map(str::to_string))
        .filter(|token| !token.is_empty())
}

fn new_session_id() -> String {
    let mut random = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut random);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
}

fn sign_token(id: &str) -> String {
    format!("{id}.{}", token_signature(id))
}

fn verified_id(token: &str) -> Option<String> {
    let (id, signature) = token.rsplit_once('.')?;
    (!id.is_empty() && constant_time_eq(signature.as_bytes(), token_signature(id).as_bytes()))
        .then(|| id.to_string())
}

fn token_signature(id: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(session_secret().as_bytes())
        .expect("session secret accepts any size");
    mac.update(id.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn session_secret() -> &'static str {
    if let Some(secret) = namix::config::session_secret() {
        return secret;
    }
    static DEVELOPMENT_SECRET: OnceLock<String> = OnceLock::new();
    DEVELOPMENT_SECRET
        .get_or_init(|| {
            let mut random = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut random);
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
        })
        .as_str()
}

fn session_cookie_secure() -> bool {
    namix::config::session_cookie_secure()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let len = left.len().max(right.len());
    for index in 0..len {
        diff |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> User {
        User {
            id: 42,
            username: "alice".into(),
            password_hash: "ignored".into(),
            name: "Alice".into(),
            is_vip: false,
            email_verified_at: None,
            created_at: jiff::Timestamp::UNIX_EPOCH,
            updated_at: jiff::Timestamp::UNIX_EPOCH,
            posts: Default::default(),
            notes: Default::default(),
            profile: Default::default(),
        }
    }

    #[test]
    fn issued_sessions_are_signed_and_rotated() {
        let service = SessionService::new();
        let first = service.issue(&user());
        assert!(service.resolve(&first).is_some());
        let second = service.rotate(Some(&first), &user());
        assert_ne!(first, second);
        assert!(service.resolve(&first).is_none());
        assert!(service.resolve(&second).is_some());
    }

    #[test]
    fn expired_or_modified_tokens_are_rejected() {
        let service = SessionService::new();
        let expired = service.issue_for(&user(), Duration::ZERO);
        assert!(service.resolve(&expired).is_none());
        let valid = service.issue(&user());
        assert!(service.resolve(&format!("{valid}x")).is_none());
    }
}
