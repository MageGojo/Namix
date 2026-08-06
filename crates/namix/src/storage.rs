//! File-storage abstraction with local-disk and S3-compatible URL contracts.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemporaryUrl {
    pub url: String,
    pub expires_at: u64,
}

/// Storage errors keep the operation and the underlying I/O source intact.
/// Callers can map `InvalidKey` and upload-policy variants to a 4xx response,
/// while `Io`/`Backend` become a logged [`crate::AppError::internal`] error.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("invalid storage key")]
    InvalidKey,
    #[error("upload exceeds {max_bytes} bytes")]
    UploadTooLarge { max_bytes: usize },
    #[error("upload extension is not allowed")]
    ExtensionNotAllowed,
    #[error("storage I/O failed")]
    Io(#[source] std::io::Error),
    #[error("storage clock failed")]
    Clock(#[source] std::time::SystemTimeError),
    #[error("storage backend failed: {message}")]
    Backend { message: String },
}

pub type StorageResult<T> = Result<T, StorageError>;

impl StorageError {
    /// Use this adapter for SDKs whose error type is not available in the
    /// framework dependency graph. Native framework drivers should preserve
    /// the concrete source with a dedicated variant instead.
    pub fn backend(message: impl Into<String>) -> Self {
        Self::Backend {
            message: message.into(),
        }
    }
}

impl From<StorageError> for crate::AppError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::InvalidKey => Self::bad_request("invalid storage key"),
            StorageError::UploadTooLarge { max_bytes } => {
                Self::validation("file", format!("upload exceeds {max_bytes} bytes"))
            }
            StorageError::ExtensionNotAllowed => {
                Self::validation("file", "upload extension is not allowed")
            }
            other => Self::internal(other),
        }
    }
}

/// Enforced before a file reaches any storage driver.
#[derive(Clone, Debug)]
pub struct UploadPolicy {
    pub max_bytes: usize,
    pub allowed_extensions: Vec<String>,
}

impl UploadPolicy {
    pub fn validate(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        if bytes.len() > self.max_bytes {
            return Err(StorageError::UploadTooLarge {
                max_bytes: self.max_bytes,
            });
        }
        if !self.allowed_extensions.is_empty() {
            let extension = Path::new(key)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !self.allowed_extensions.iter().any(|allowed| {
                allowed
                    .trim_start_matches('.')
                    .eq_ignore_ascii_case(extension)
            }) {
                return Err(StorageError::ExtensionNotAllowed);
            }
        }
        Ok(())
    }
}

pub trait StorageDriver: Send + Sync + 'static {
    fn put(&self, key: &str, bytes: &[u8]) -> StorageResult<()>;
    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>>;
    fn delete(&self, key: &str) -> StorageResult<()>;
    fn url(&self, key: &str) -> String;
    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl>;
}

#[derive(Clone)]
pub struct Storage {
    driver: Arc<dyn StorageDriver>,
}

impl Storage {
    pub fn new(driver: impl StorageDriver) -> Self {
        Self {
            driver: Arc::new(driver),
        }
    }

    pub fn put(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        self.driver.put(key, bytes)
    }

    pub fn put_with_policy(
        &self,
        key: &str,
        bytes: &[u8],
        policy: &UploadPolicy,
    ) -> StorageResult<()> {
        policy.validate(key, bytes)?;
        self.put(key, bytes)
    }

    pub fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        self.driver.get(key)
    }

    pub fn delete(&self, key: &str) -> StorageResult<()> {
        self.driver.delete(key)
    }

    pub fn url(&self, key: &str) -> String {
        self.driver.url(key)
    }

    pub fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        self.driver.temporary_url(key, ttl)
    }
}

#[derive(Clone, Debug)]
pub struct LocalStorage {
    root: PathBuf,
    public_base: String,
}

impl LocalStorage {
    pub fn new(root: impl Into<PathBuf>, public_base: impl Into<String>) -> Self {
        Self {
            root: root.into(),
            public_base: public_base.into().trim_end_matches('/').into(),
        }
    }

    fn path(&self, key: &str) -> StorageResult<PathBuf> {
        let path = Path::new(key);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(StorageError::InvalidKey);
        }
        Ok(self.root.join(path))
    }
}

impl StorageDriver for LocalStorage {
    fn put(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        let path = self.path(key)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(StorageError::Io)?;
        }
        fs::write(path, bytes).map_err(StorageError::Io)
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let path = self.path(key)?;
        match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(StorageError::Io(error)),
        }
    }

    fn delete(&self, key: &str) -> StorageResult<()> {
        let path = self.path(key)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(StorageError::Io(error)),
        }
    }

    fn url(&self, key: &str) -> String {
        format!("{}/{}", self.public_base, key.trim_start_matches('/'))
    }

    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        let expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(StorageError::Clock)?
            .as_secs()
            + ttl.as_secs();
        Ok(TemporaryUrl {
            url: format!("{}?expires={expires_at}", self.url(key)),
            expires_at,
        })
    }
}

/// S3-compatible adapter boundary. Applications supply a concrete transport
/// (AWS SDK, MinIO, R2, etc.) while controllers keep the same `Storage` API.
pub trait S3Transport: Send + Sync + 'static {
    fn put_object(&self, bucket: &str, key: &str, bytes: &[u8]) -> StorageResult<()>;
    fn get_object(&self, bucket: &str, key: &str) -> StorageResult<Option<Vec<u8>>>;
    fn delete_object(&self, bucket: &str, key: &str) -> StorageResult<()>;
    fn presign_get(&self, bucket: &str, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl>;
}

#[derive(Clone)]
pub struct S3CompatibleStorage<T> {
    transport: Arc<T>,
    bucket: String,
    endpoint: String,
}

impl<T: S3Transport> S3CompatibleStorage<T> {
    pub fn new(transport: T, bucket: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            transport: Arc::new(transport),
            bucket: bucket.into(),
            endpoint: endpoint.into().trim_end_matches('/').into(),
        }
    }
}

impl<T: S3Transport> StorageDriver for S3CompatibleStorage<T> {
    fn put(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        self.transport.put_object(&self.bucket, key, bytes)
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        self.transport.get_object(&self.bucket, key)
    }

    fn delete(&self, key: &str) -> StorageResult<()> {
        self.transport.delete_object(&self.bucket, key)
    }

    fn url(&self, key: &str) -> String {
        format!(
            "{}/{}/{}",
            self.endpoint,
            self.bucket,
            key.trim_start_matches('/')
        )
    }

    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        self.transport.presign_get(&self.bucket, key, ttl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_storage_blocks_traversal() {
        let dir = std::env::temp_dir().join("namix-storage-test");
        let storage = LocalStorage::new(&dir, "/files");
        assert!(matches!(
            storage.put("../../x", b"no"),
            Err(StorageError::InvalidKey)
        ));
        storage.put("a/b.txt", b"ok").unwrap();
        assert_eq!(storage.get("a/b.txt").unwrap(), Some(b"ok".to_vec()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn upload_policy_has_machine_readable_errors() {
        let policy = UploadPolicy {
            max_bytes: 2,
            allowed_extensions: vec!["png".into()],
        };
        assert!(matches!(
            policy.validate("avatar.jpg", b"ok"),
            Err(StorageError::ExtensionNotAllowed)
        ));
        assert!(matches!(
            policy.validate("avatar.png", b"too large"),
            Err(StorageError::UploadTooLarge { max_bytes: 2 })
        ));
    }

    #[test]
    fn upload_errors_map_to_field_or_internal_app_errors() {
        let invalid: crate::AppError = StorageError::InvalidKey.into();
        assert_eq!(invalid.status().as_u16(), 400);
        let io: crate::AppError =
            StorageError::Io(std::io::Error::other("disk unavailable")).into();
        assert_eq!(io.status().as_u16(), 500);
        assert!(std::error::Error::source(&io).is_some());
    }
}
