//! 会话：签名 Cookie + 可选 JWT Bearer → 过期、轮换和全设备撤销。
//!
//! - Cookie 携带不透明 HMAC token（`{id}.{sig}`），不包含用户名
//! - API 可用同一会话签发的 HS256 JWT（`Authorization: Bearer …`）
//! - JWT 含 `sid`，登出/全设备撤销仍走 [`namix::SessionStore`]
//! - 时长来自 `[session] lifetime_secs` / `jwt_lifetime_secs`（可在签发时覆盖）

use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine as _;
use hmac::{Hmac, Mac};
use namix::{AppError, AuthSession, CookieOptions, Jwt, JwtClaims, Session};
use rand::RngCore;
use sha2::Sha256;

use crate::models::user::User;

/// 浏览器会话 Cookie（值 = 签名 token，不是用户名）。
pub const SESSION_COOKIE: &str = "namix_session";
/// 旧版误用 username 的 Cookie，登录/登出时顺带清掉。
pub const LEGACY_SESSION_COOKIE: &str = "namix_user";
/// 配置缺失时的 Cookie / opaque 默认时长（7 天）。
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

/// 一次登录同时给出 Cookie token 与 JWT access token。
#[derive(Clone, Debug)]
pub struct IssuedTokens {
    /// 写入 `namix_session` Cookie 的不透明签名 token。
    pub cookie_token: String,
    /// `Authorization: Bearer` 用的 HS256 JWT。
    pub access_token: String,
    /// JWT 剩余秒数（`expires_in`）。
    pub expires_in: u64,
}

#[derive(Clone, Default)]
pub struct SessionService;

impl SessionService {
    pub fn new() -> Self {
        Self
    }

    fn store(&self) -> Session {
        namix::session::current()
    }

    /// Cookie / opaque 会话时长（`[session].lifetime_secs`）。
    pub fn lifetime() -> Duration {
        namix::config::session_lifetime()
    }

    /// JWT access token 时长（`[session].jwt_lifetime_secs`）。
    pub fn jwt_lifetime() -> Duration {
        namix::config::jwt_lifetime()
    }

    /// 签发有绝对过期时间的全新会话。返回值可直接写入 Cookie。
    pub fn issue(&self, user: &User) -> Result<String, AppError> {
        self.issue_for(user, Self::lifetime())
    }

    pub fn issue_for(&self, user: &User, ttl: Duration) -> Result<String, AppError> {
        let id = new_session_id();
        self.put_session(&id, user, ttl)?;
        Ok(sign_token(&id))
    }

    /// Cookie token + JWT access token（共享同一 `sid` / Store 记录）。
    pub fn issue_pair(&self, user: &User) -> Result<IssuedTokens, AppError> {
        self.issue_pair_for(user, Self::lifetime(), Self::jwt_lifetime())
    }

    pub fn issue_pair_for(
        &self,
        user: &User,
        session_ttl: Duration,
        jwt_ttl: Duration,
    ) -> Result<IssuedTokens, AppError> {
        let id = new_session_id();
        let record = self.put_session(&id, user, session_ttl)?;
        let cookie_token = sign_token(&id);
        let access_token = encode_jwt(&id, &record, jwt_ttl)?;
        Ok(IssuedTokens {
            cookie_token,
            access_token,
            expires_in: jwt_ttl.as_secs().min(
                record
                    .expires_at_unix
                    .saturating_sub(now_unix()),
            ),
        })
    }

    /// 登录/重新认证时轮换：旧会话立即失效，返回新的 Cookie + JWT。
    pub fn rotate_pair(
        &self,
        previous_token: Option<&str>,
        user: &User,
    ) -> Result<IssuedTokens, AppError> {
        if let Some(previous_token) = previous_token {
            self.revoke(previous_token)?;
        }
        self.issue_pair(user)
    }

    /// 登录/重新认证时轮换 token，旧会话立即失效（仅 Cookie opaque）。
    pub fn rotate(
        &self,
        previous_token: Option<&str>,
        user: &User,
    ) -> Result<String, AppError> {
        if let Some(previous_token) = previous_token {
            self.revoke(previous_token)?;
        }
        self.issue(user)
    }

    /// 为已有 opaque Cookie token 补发 JWT（不新建会话）。
    pub fn access_token_for(
        &self,
        cookie_token: &str,
    ) -> Result<Option<IssuedTokens>, AppError> {
        let Some(id) = verified_id(cookie_token) else {
            return Ok(None);
        };
        let Some(record) = self.store().get(&id)? else {
            return Ok(None);
        };
        let jwt_ttl = Self::jwt_lifetime();
        let access_token = encode_jwt(&id, &record, jwt_ttl)?;
        Ok(Some(IssuedTokens {
            cookie_token: cookie_token.to_string(),
            access_token,
            expires_in: jwt_ttl.as_secs().min(
                record
                    .expires_at_unix
                    .saturating_sub(now_unix()),
            ),
        }))
    }

    /// 登出：销毁一个会话（opaque 或 JWT 均可）。
    pub fn revoke(&self, token: &str) -> Result<(), AppError> {
        if let Some(id) = self.session_id_of(token) {
            self.store().forget(&id)?;
        }
        Ok(())
    }

    /// 当前密码重置、主动安全退出等场景使用：撤销该用户的所有设备。
    pub fn revoke_all_for_user(&self, user_id: u64) -> Result<usize, AppError> {
        self.store().forget_user(user_id).map_err(AppError::from)
    }

    /// Cookie opaque 或 JWT Bearer → [`LoginUser`]。过期 / 已撤销均返回 `None`。
    pub fn resolve(&self, token: &str) -> Result<Option<LoginUser>, AppError> {
        let Some(id) = self.session_id_of_valid(token) else {
            return Ok(None);
        };
        let Some(record) = self.store().get(&id)? else {
            return Ok(None);
        };
        Ok(Some(LoginUser {
            id: record.user_id,
            username: record.username,
            is_vip: record.is_vip,
            session_id: id,
        }))
    }

    pub fn cookie_options() -> CookieOptions {
        Self::cookie_options_for(Self::lifetime())
    }

    /// 自定义 Cookie `Max-Age`（秒），例如「记住我」更长会话。
    pub fn cookie_options_for(ttl: Duration) -> CookieOptions {
        CookieOptions {
            secure: session_cookie_secure(),
            max_age: Some(ttl.as_secs()),
            ..CookieOptions::default()
        }
    }

    fn put_session(
        &self,
        id: &str,
        user: &User,
        ttl: Duration,
    ) -> Result<AuthSession, AppError> {
        let record = AuthSession::with_ttl(user.id, user.username.clone(), user.is_vip, ttl);
        self.store().put(id, &record)?;
        Ok(record)
    }

    fn session_id_of(&self, token: &str) -> Option<String> {
        if Jwt::looks_like(token) {
            Jwt::decode_ignore_exp(token, session_secret())
                .ok()
                .map(|claims| claims.sid)
        } else {
            verified_id(token)
        }
    }

    fn session_id_of_valid(&self, token: &str) -> Option<String> {
        if Jwt::looks_like(token) {
            let claims = Jwt::decode(token, session_secret()).ok()?;
            Some(claims.sid)
        } else {
            verified_id(token)
        }
    }
}

/// 从 Cookie 或 `Authorization: Bearer <opaque|jwt>` 取 token。
pub fn session_id_from(req: &namix::Request) -> Option<String> {
    req.bearer()
        .map(str::to_string)
        .or_else(|| req.cookie(SESSION_COOKIE).map(str::to_string))
        .filter(|token| !token.is_empty())
}

fn encode_jwt(
    session_id: &str,
    record: &AuthSession,
    jwt_ttl: Duration,
) -> Result<String, AppError> {
    let claims = JwtClaims::from_session(session_id, record, jwt_ttl);
    Jwt::encode(&claims, session_secret()).map_err(AppError::from)
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
    if Jwt::looks_like(token) {
        return None;
    }
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

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
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
    use namix::{MemorySessionStore, Session};

    fn install_memory() {
        namix::session::install(Session::new(MemorySessionStore::default()));
    }

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
        install_memory();
        let service = SessionService::new();
        let first = service.issue(&user()).unwrap();
        assert!(service.resolve(&first).unwrap().is_some());
        let second = service.rotate(Some(&first), &user()).unwrap();
        assert_ne!(first, second);
        assert!(service.resolve(&first).unwrap().is_none());
        assert!(service.resolve(&second).unwrap().is_some());
    }

    #[test]
    fn expired_or_modified_tokens_are_rejected() {
        install_memory();
        let service = SessionService::new();
        let expired = service.issue_for(&user(), Duration::ZERO).unwrap();
        assert!(service.resolve(&expired).unwrap().is_none());
        let valid = service.issue(&user()).unwrap();
        assert!(service.resolve(&format!("{valid}x")).unwrap().is_none());
    }

    #[test]
    fn jwt_bearer_resolves_and_revokes_with_cookie_session() {
        install_memory();
        let service = SessionService::new();
        let tokens = service
            .issue_pair_for(
                &user(),
                Duration::from_secs(3600),
                Duration::from_secs(600),
            )
            .unwrap();
        assert!(Jwt::looks_like(&tokens.access_token));
        assert_eq!(
            service
                .resolve(&tokens.access_token)
                .unwrap()
                .unwrap()
                .username,
            "alice"
        );
        assert_eq!(
            service
                .resolve(&tokens.cookie_token)
                .unwrap()
                .unwrap()
                .session_id,
            service
                .resolve(&tokens.access_token)
                .unwrap()
                .unwrap()
                .session_id
        );
        service.revoke(&tokens.access_token).unwrap();
        assert!(service.resolve(&tokens.cookie_token).unwrap().is_none());
        assert!(service.resolve(&tokens.access_token).unwrap().is_none());
    }

    #[test]
    fn cookie_options_honor_custom_ttl() {
        let options = SessionService::cookie_options_for(Duration::from_secs(120));
        assert_eq!(options.max_age, Some(120));
    }
}
