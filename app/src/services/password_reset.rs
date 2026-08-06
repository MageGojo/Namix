//! 一次性密码重置令牌。
//!
//! 默认内存实现适合开发；生产驱动可把同一令牌语义存到数据库或 Redis。

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use rand::RngCore;

const RESET_TTL: Duration = Duration::from_secs(60 * 30);

#[derive(Clone, Debug)]
struct ResetRecord {
    user_id: u64,
    expires_at: SystemTime,
}

type Store = RwLock<HashMap<String, ResetRecord>>;

fn store() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(HashMap::new()))
}

#[derive(Clone, Default)]
pub struct PasswordResetService;

impl PasswordResetService {
    pub fn issue(&self, user_id: u64) -> String {
        let token = new_token();
        let expires_at = SystemTime::now().checked_add(RESET_TTL).unwrap_or(SystemTime::now());
        let mut guard = store().write().expect("password reset store");
        // 每位用户仅保留一个活跃令牌，新的申请使旧邮件立即失效。
        guard.retain(|_, record| record.user_id != user_id && record.expires_at > SystemTime::now());
        guard.insert(
            token.clone(),
            ResetRecord {
                user_id,
                expires_at,
            },
        );
        token
    }

    /// Consume first, then return the bound user.  A token cannot be replayed
    /// even if the password write later fails.
    pub fn consume(&self, token: &str) -> Option<u64> {
        let mut guard = store().write().expect("password reset store");
        let record = guard.remove(token)?;
        (record.expires_at > SystemTime::now()).then_some(record.user_id)
    }
}

fn new_token() -> String {
    let mut random = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut random);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_tokens_are_one_time() {
        let service = PasswordResetService;
        let token = service.issue(42);
        assert_eq!(service.consume(&token), Some(42));
        assert_eq!(service.consume(&token), None);
    }
}
