//! Compact HS256 JWT for API Bearer tokens.
//!
//! Browser sessions stay on opaque signed cookies. JWT access tokens share the
//! same [`crate::config::session_secret`] and carry `sid` so logout / revoke-all
//! still works through [`crate::SessionStore`].

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;

use crate::session::AuthSession;

type HmacSha256 = Hmac<Sha256>;

const JWT_HEADER_JSON: &str = r#"{"alg":"HS256","typ":"JWT"}"#;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum JwtError {
    #[error("jwt malformed")]
    Malformed,
    #[error("jwt signature invalid")]
    BadSignature,
    #[error("jwt expired")]
    Expired,
    #[error("jwt encode/decode failed: {0}")]
    Codec(String),
    #[error("jwt signing secret missing")]
    MissingSecret,
}

impl From<JwtError> for crate::AppError {
    fn from(error: JwtError) -> Self {
        match error {
            JwtError::Malformed | JwtError::BadSignature | JwtError::Expired => {
                Self::Unauthenticated
            }
            other => Self::internal(other),
        }
    }
}

/// Claims embedded in Namix access tokens.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JwtClaims {
    /// User id (string for JWT `sub` convention).
    pub sub: String,
    /// Opaque session id — lookup key in [`crate::SessionStore`].
    pub sid: String,
    pub username: String,
    pub is_vip: bool,
    pub iat: u64,
    pub exp: u64,
}

impl JwtClaims {
    pub fn from_session(session_id: &str, session: &AuthSession, ttl: Duration) -> Self {
        let now = now_secs();
        let exp = now
            .saturating_add(ttl.as_secs())
            .min(session.expires_at_unix);
        Self {
            sub: session.user_id.to_string(),
            sid: session_id.to_string(),
            username: session.username.clone(),
            is_vip: session.is_vip,
            iat: now,
            exp,
        }
    }

    pub fn user_id(&self) -> Option<u64> {
        self.sub.parse().ok()
    }

    pub fn is_expired(&self) -> bool {
        now_secs() >= self.exp
    }
}

/// Encode / decode HS256 JWTs with the process session secret.
pub struct Jwt;

impl Jwt {
    pub fn looks_like(token: &str) -> bool {
        let mut dots = 0usize;
        for ch in token.chars() {
            if ch == '.' {
                dots += 1;
                if dots > 2 {
                    return false;
                }
            }
        }
        dots == 2 && token.starts_with("eyJ")
    }

    pub fn encode(claims: &JwtClaims, secret: &str) -> Result<String, JwtError> {
        if secret.is_empty() {
            return Err(JwtError::MissingSecret);
        }
        let header = b64url(JWT_HEADER_JSON.as_bytes());
        let payload = b64url(
            &serde_json::to_vec(claims).map_err(|error| JwtError::Codec(error.to_string()))?,
        );
        let signing_input = format!("{header}.{payload}");
        let signature = sign(secret.as_bytes(), signing_input.as_bytes())?;
        Ok(format!("{signing_input}.{}", b64url(&signature)))
    }

    pub fn decode(token: &str, secret: &str) -> Result<JwtClaims, JwtError> {
        Self::decode_inner(token, secret, true)
    }

    /// Decode even when `exp` has passed — used for logout / revoke.
    pub fn decode_ignore_exp(token: &str, secret: &str) -> Result<JwtClaims, JwtError> {
        Self::decode_inner(token, secret, false)
    }

    fn decode_inner(token: &str, secret: &str, enforce_exp: bool) -> Result<JwtClaims, JwtError> {
        if secret.is_empty() {
            return Err(JwtError::MissingSecret);
        }
        let mut parts = token.split('.');
        let header = parts.next().ok_or(JwtError::Malformed)?;
        let payload = parts.next().ok_or(JwtError::Malformed)?;
        let signature = parts.next().ok_or(JwtError::Malformed)?;
        if parts.next().is_some() || header.is_empty() || payload.is_empty() || signature.is_empty()
        {
            return Err(JwtError::Malformed);
        }

        let signing_input = format!("{header}.{payload}");
        let expected = sign(secret.as_bytes(), signing_input.as_bytes())?;
        let actual = b64url_decode(signature)?;
        if !constant_time_eq(&expected, &actual) {
            return Err(JwtError::BadSignature);
        }

        let claims: JwtClaims = serde_json::from_slice(&b64url_decode(payload)?)
            .map_err(|error| JwtError::Codec(error.to_string()))?;
        if enforce_exp && claims.is_expired() {
            return Err(JwtError::Expired);
        }
        Ok(claims)
    }
}

fn sign(secret: &[u8], input: &[u8]) -> Result<Vec<u8>, JwtError> {
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|error| JwtError::Codec(format!("hmac key: {error}")))?;
    mac.update(input);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn b64url_decode(value: &str) -> Result<Vec<u8>, JwtError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| JwtError::Malformed)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
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

    #[test]
    fn round_trips_and_rejects_tampering() {
        let session = AuthSession::with_ttl(7, "alice", true, Duration::from_secs(3600));
        let claims = JwtClaims::from_session("sid-1", &session, Duration::from_secs(600));
        let token = Jwt::encode(&claims, "test-secret").unwrap();
        assert!(Jwt::looks_like(&token));
        let decoded = Jwt::decode(&token, "test-secret").unwrap();
        assert_eq!(decoded.sid, "sid-1");
        assert_eq!(decoded.username, "alice");

        let mut bad = token.clone();
        bad.push('x');
        let tampered = Jwt::decode(&bad, "test-secret");
        assert!(
            matches!(
                tampered,
                Err(JwtError::Malformed) | Err(JwtError::BadSignature)
            ),
            "unexpected tamper result: {tampered:?}"
        );
        assert!(Jwt::decode(&token, "other-secret").is_err());
    }

    #[test]
    fn expired_tokens_fail_unless_ignored() {
        let mut claims = JwtClaims {
            sub: "1".into(),
            sid: "s".into(),
            username: "a".into(),
            is_vip: false,
            iat: 1,
            exp: 2,
        };
        let token = Jwt::encode(&claims, "secret").unwrap();
        assert_eq!(Jwt::decode(&token, "secret"), Err(JwtError::Expired));
        claims = Jwt::decode_ignore_exp(&token, "secret").unwrap();
        assert_eq!(claims.sid, "s");
    }
}
