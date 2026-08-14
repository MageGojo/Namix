//! 邮箱验证一次性令牌。

use std::sync::OnceLock;
use std::time::Duration;

use namix::prelude::*;
use namix::OneTimeTokenStore;

use crate::services::user::UserService;

const PURPOSE: &str = "email_verify";
const VERIFY_TTL: Duration = Duration::from_secs(60 * 60 * 24);

fn store() -> &'static OneTimeTokenStore {
    static STORE: OnceLock<OneTimeTokenStore> = OnceLock::new();
    STORE.get_or_init(|| {
        if cfg!(test) {
            return OneTimeTokenStore::memory();
        }
        OneTimeTokenStore::file("./storage/email_verify.json").unwrap_or_else(|error| {
            namix::log::warn!(
                "email verify file store unavailable ({error}); falling back to memory"
            );
            OneTimeTokenStore::memory()
        })
    })
}

#[derive(Clone, Default)]
pub struct EmailVerificationService;

impl EmailVerificationService {
    pub fn issue(&self, user_id: u64) -> String {
        store()
            .issue(PURPOSE, user_id, VERIFY_TTL)
            .expect("email verify token issue")
    }

    pub fn consume(&self, token: &str) -> Option<u64> {
        store().consume(PURPOSE, token).ok().flatten()
    }

    pub fn notify(&self, user_id: u64, email: &str) -> Result<(), AppError> {
        if email.trim().is_empty() {
            return Ok(());
        }
        let token = self.issue(user_id);
        let link = format!("/email/verify?token={token}");
        Mail::send(
            MailMessage::new(email, "验证你的 Namix 邮箱").text(format!(
                "打开此链接完成邮箱验证（24 小时内有效）：{link}"
            )),
        )?;
        Ok(())
    }

    pub async fn verify(&self, token: &str) -> Result<u64, AppError> {
        let user_id = self
            .consume(token)
            .ok_or_else(|| AppError::validation("token", "token.invalid"))?;
        UserService::new().mark_email_verified(user_id).await?;
        Ok(user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_tokens_are_one_time() {
        let service = EmailVerificationService;
        let token = service.issue(7);
        assert_eq!(service.consume(&token), Some(7));
        assert_eq!(service.consume(&token), None);
    }
}
