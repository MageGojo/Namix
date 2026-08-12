//! Application-level encryption (Laravel `Crypt`-style, AES-256-GCM).
//!
//! Values are sealed with a key derived from [`crate::config::session_secret`]
//! via HKDF. Decryption is server-only — never ship ciphertext for the browser
//! to "decode" into roles or session identity.

use std::sync::OnceLock;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;

const MAGIC: &str = "nx1:";
const HKDF_INFO: &[u8] = b"namix-crypt-v1";
const NONCE_LEN: usize = 12;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptError {
    #[error("crypt key is not installed (Boot did not call Crypt::install)")]
    NotInstalled,
    #[error("crypt payload malformed")]
    Malformed,
    #[error("crypt decrypt failed")]
    DecryptFailed,
    #[error("crypt encrypt failed")]
    EncryptFailed,
}

impl From<CryptError> for crate::AppError {
    fn from(error: CryptError) -> Self {
        match error {
            CryptError::Malformed | CryptError::DecryptFailed => {
                Self::bad_request("encrypted payload is invalid")
            }
            other => Self::internal(other),
        }
    }
}

static KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// AES-256-GCM encrypt / decrypt for cookies, flash, and server-only blobs.
pub struct Crypt;

impl Crypt {
    /// Install the process key (derived from the session secret).
    pub fn install(secret: &str) {
        let key = derive_key(secret.as_bytes());
        let _ = KEY.set(key);
    }

    pub fn is_installed() -> bool {
        KEY.get().is_some()
    }

    /// Whether `value` looks like a Namix sealed payload.
    pub fn is_sealed(value: &str) -> bool {
        value.starts_with(MAGIC)
    }

    pub fn encrypt(plaintext: &[u8]) -> Result<String, CryptError> {
        let key = KEY.get().ok_or(CryptError::NotInstalled)?;
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptError::EncryptFailed)?;
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|_| CryptError::EncryptFailed)?;
        let mut packed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        packed.extend_from_slice(&nonce_bytes);
        packed.extend_from_slice(&ciphertext);
        Ok(format!(
            "{MAGIC}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(packed)
        ))
    }

    pub fn encrypt_str(plaintext: &str) -> Result<String, CryptError> {
        Self::encrypt(plaintext.as_bytes())
    }

    pub fn decrypt(sealed: &str) -> Result<Vec<u8>, CryptError> {
        let key = KEY.get().ok_or(CryptError::NotInstalled)?;
        let body = sealed.strip_prefix(MAGIC).ok_or(CryptError::Malformed)?;
        let packed = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_| CryptError::Malformed)?;
        if packed.len() <= NONCE_LEN {
            return Err(CryptError::Malformed);
        }
        let (nonce_bytes, ciphertext) = packed.split_at(NONCE_LEN);
        let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptError::DecryptFailed)?;
        cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
            .map_err(|_| CryptError::DecryptFailed)
    }

    pub fn decrypt_str(sealed: &str) -> Result<String, CryptError> {
        let bytes = Self::decrypt(sealed)?;
        String::from_utf8(bytes).map_err(|_| CryptError::Malformed)
    }

    /// Encrypt when Crypt is installed; otherwise return plaintext unchanged.
    /// Used by flash cookies so development still works before Boot installs a key.
    pub fn seal_cookie_value(plaintext: &str) -> String {
        Self::encrypt_str(plaintext).unwrap_or_else(|_| plaintext.to_string())
    }

    /// Decrypt a sealed value, or pass through legacy plaintext flash payloads.
    pub fn open_cookie_value(raw: &str) -> String {
        if Self::is_sealed(raw) {
            Self::decrypt_str(raw).unwrap_or_default()
        } else {
            raw.to_string()
        }
    }
}

fn derive_key(secret: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(b"namix-crypt"), secret);
    let mut key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut key)
        .expect("HKDF expand length is valid");
    key
}

/// Flash / cookie seal hooks used by `namix-http` without a circular dependency
/// on the higher-level Crypt API surface.
pub fn install_http_cookie_crypt() {
    namix_http::crypt::install(Crypt::seal_cookie_value, Crypt::open_cookie_value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_tampering() {
        Crypt::install("unit-test-crypt-secret");
        let sealed = Crypt::encrypt_str("hello flash").expect("encrypt");
        assert!(Crypt::is_sealed(&sealed));
        assert_eq!(Crypt::decrypt_str(&sealed).unwrap(), "hello flash");
        let mut bad = sealed;
        bad.push('x');
        assert!(Crypt::decrypt_str(&bad).is_err());
    }

    #[test]
    fn open_passthrough_keeps_legacy_flash() {
        assert_eq!(Crypt::open_cookie_value("ok"), "ok");
        assert_eq!(Crypt::open_cookie_value("e:hi"), "e:hi");
    }
}
