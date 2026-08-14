//! 一次性密码重置令牌。
//!
//! 开发测试走内存；运行时默认落盘 `storage/password_reset.json`，重启后仍可消费。

use std::sync::OnceLock;
use std::time::Duration;

use namix::OneTimeTokenStore;

const PURPOSE: &str = "password_reset";
const RESET_TTL: Duration = Duration::from_secs(60 * 30);

fn store() -> &'static OneTimeTokenStore {
    static STORE: OnceLock<OneTimeTokenStore> = OnceLock::new();
    STORE.get_or_init(|| {
        if cfg!(test) {
            return OneTimeTokenStore::memory();
        }
        OneTimeTokenStore::file("./storage/password_reset.json").unwrap_or_else(|error| {
            namix::log::warn!(
                "password reset file store unavailable ({error}); falling back to memory"
            );
            OneTimeTokenStore::memory()
        })
    })
}

#[derive(Clone, Default)]
pub struct PasswordResetService;

impl PasswordResetService {
    pub fn issue(&self, user_id: u64) -> String {
        store()
            .issue(PURPOSE, user_id, RESET_TTL)
            .expect("password reset token issue")
    }

    /// Consume first, then return the bound user.  A token cannot be replayed
    /// even if the password write later fails.
    pub fn consume(&self, token: &str) -> Option<u64> {
        store().consume(PURPOSE, token).ok().flatten()
    }
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
