//! One-time tokens (password reset, email verify) with memory or file backends.
//!
//! Tokens are stored as SHA-256 digests. Issuing a new token for the same
//! `(purpose, user_id)` invalidates previous unused tokens.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TokenStoreError {
    #[error("one-time token store lock poisoned")]
    LockPoisoned,
    #[error("one-time token store I/O failed")]
    Io(#[from] std::io::Error),
    #[error("one-time token store serialization failed")]
    Serialize(#[from] serde_json::Error),
}

impl From<TokenStoreError> for crate::AppError {
    fn from(error: TokenStoreError) -> Self {
        Self::internal(error)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Record {
    purpose: String,
    user_id: u64,
    expires_at_unix: u64,
}

enum Backend {
    Memory(Mutex<HashMap<String, Record>>),
    File { path: PathBuf, lock: Mutex<()> },
}

/// Process-local or file-backed one-time token store.
pub struct OneTimeTokenStore {
    backend: Backend,
}

impl OneTimeTokenStore {
    pub fn memory() -> Self {
        Self {
            backend: Backend::Memory(Mutex::new(HashMap::new())),
        }
    }

    pub fn file(path: impl AsRef<Path>) -> Result<Self, TokenStoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            fs::write(&path, "{}")?;
        }
        Ok(Self {
            backend: Backend::File {
                path,
                lock: Mutex::new(()),
            },
        })
    }

    /// Shared process store for development. Production should construct
    /// [`Self::file`] (or a future Redis driver) explicitly.
    pub fn process_memory() -> &'static Self {
        static STORE: OnceLock<OneTimeTokenStore> = OnceLock::new();
        STORE.get_or_init(Self::memory)
    }

    pub fn issue(
        &self,
        purpose: &str,
        user_id: u64,
        ttl: Duration,
    ) -> Result<String, TokenStoreError> {
        let token = new_token();
        let digest = digest_token(&token);
        let expires_at_unix = unix_secs().saturating_add(ttl.as_secs());
        let record = Record {
            purpose: purpose.to_string(),
            user_id,
            expires_at_unix,
        };
        self.mutate(|map| {
            let now = unix_secs();
            map.retain(|_, item| {
                item.expires_at_unix > now && !(item.purpose == purpose && item.user_id == user_id)
            });
            map.insert(digest, record);
        })?;
        Ok(token)
    }

    pub fn consume(&self, purpose: &str, token: &str) -> Result<Option<u64>, TokenStoreError> {
        let digest = digest_token(token);
        self.mutate(|map| {
            let record = map.remove(&digest)?;
            if record.purpose != purpose || record.expires_at_unix <= unix_secs() {
                return None;
            }
            Some(record.user_id)
        })
    }

    fn mutate<T>(
        &self,
        f: impl FnOnce(&mut HashMap<String, Record>) -> T,
    ) -> Result<T, TokenStoreError> {
        match &self.backend {
            Backend::Memory(lock) => {
                let mut guard = lock.lock().map_err(|_| TokenStoreError::LockPoisoned)?;
                Ok(f(&mut guard))
            }
            Backend::File { path, lock } => {
                let _guard = lock.lock().map_err(|_| TokenStoreError::LockPoisoned)?;
                let raw = fs::read_to_string(path)?;
                let mut map: HashMap<String, Record> =
                    serde_json::from_str(&raw).unwrap_or_default();
                let result = f(&mut map);
                let encoded = serde_json::to_string(&map)?;
                atomic_write(path, encoded.as_bytes())?;
                Ok(result)
            }
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), TokenStoreError> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn new_token() -> String {
    let mut random = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut random);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
}

fn digest_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_tokens_are_one_time_and_replace_per_user() {
        let store = OneTimeTokenStore::memory();
        let first = store
            .issue("password_reset", 9, Duration::from_secs(60))
            .unwrap();
        let second = store
            .issue("password_reset", 9, Duration::from_secs(60))
            .unwrap();
        assert_eq!(store.consume("password_reset", &first).unwrap(), None);
        assert_eq!(store.consume("password_reset", &second).unwrap(), Some(9));
        assert_eq!(store.consume("password_reset", &second).unwrap(), None);
    }

    #[test]
    fn file_tokens_survive_reopen() {
        let dir =
            std::env::temp_dir().join(format!("namix-ott-{}-{}", std::process::id(), unix_secs()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.json");
        let token = {
            let store = OneTimeTokenStore::file(&path).unwrap();
            store
                .issue("password_reset", 3, Duration::from_secs(60))
                .unwrap()
        };
        let store = OneTimeTokenStore::file(&path).unwrap();
        assert_eq!(store.consume("password_reset", &token).unwrap(), Some(3));
        let _ = fs::remove_dir_all(dir);
    }
}
