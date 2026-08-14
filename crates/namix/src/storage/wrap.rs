//! Scoped prefix and read-only wrappers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::{StorageDriver, StorageError, StorageResult, TemporaryUrl, Visibility, normalize_key};

#[derive(Clone)]
pub struct ScopedStorage {
    inner: Arc<dyn StorageDriver>,
    prefix: String,
}

impl ScopedStorage {
    pub fn new(inner: Arc<dyn StorageDriver>, prefix: &str) -> StorageResult<Self> {
        let prefix = normalize_key(prefix)?.to_string();
        Ok(Self { inner, prefix })
    }

    fn full(&self, key: &str) -> StorageResult<String> {
        if key.is_empty() {
            return Ok(self.prefix.clone());
        }
        Ok(format!("{}/{}", self.prefix, normalize_key(key)?))
    }

    fn strip(&self, key: &str) -> Option<String> {
        key.strip_prefix(&self.prefix)
            .map(|rest| rest.trim_start_matches('/').to_string())
            .filter(|rest| !rest.is_empty() || key == self.prefix)
    }
}

impl StorageDriver for ScopedStorage {
    fn put(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        self.inner.put(&self.full(key)?, bytes)
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        self.inner.get(&self.full(key)?)
    }

    fn delete(&self, key: &str) -> StorageResult<()> {
        self.inner.delete(&self.full(key)?)
    }

    fn url(&self, key: &str) -> String {
        match self.full(key) {
            Ok(full) => self.inner.url(&full),
            Err(_) => String::new(),
        }
    }

    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        self.inner.temporary_url(&self.full(key)?, ttl)
    }

    fn verify_temporary_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        self.inner
            .verify_temporary_url(&self.full(key)?, expires_at, signature)
    }

    fn exists(&self, key: &str) -> StorageResult<bool> {
        self.inner.exists(&self.full(key)?)
    }

    fn copy(&self, from: &str, to: &str) -> StorageResult<()> {
        self.inner.copy(&self.full(from)?, &self.full(to)?)
    }

    fn rename(&self, from: &str, to: &str) -> StorageResult<()> {
        self.inner.rename(&self.full(from)?, &self.full(to)?)
    }

    fn prepend(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        self.inner.prepend(&self.full(key)?, bytes)
    }

    fn append(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        self.inner.append(&self.full(key)?, bytes)
    }

    fn files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        Ok(self
            .inner
            .files(&self.full(prefix)?)?
            .into_iter()
            .filter_map(|key| self.strip(&key))
            .collect())
    }

    fn all_files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        let full = if prefix.is_empty() {
            self.prefix.clone()
        } else {
            self.full(prefix)?
        };
        Ok(self
            .inner
            .all_files(&full)?
            .into_iter()
            .filter_map(|key| self.strip(&key))
            .collect())
    }

    fn directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        let full = if prefix.is_empty() {
            self.prefix.clone()
        } else {
            self.full(prefix)?
        };
        Ok(self
            .inner
            .directories(&full)?
            .into_iter()
            .filter_map(|key| self.strip(&key))
            .collect())
    }

    fn all_directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        let full = if prefix.is_empty() {
            self.prefix.clone()
        } else {
            self.full(prefix)?
        };
        Ok(self
            .inner
            .all_directories(&full)?
            .into_iter()
            .filter_map(|key| self.strip(&key))
            .collect())
    }

    fn make_directory(&self, prefix: &str) -> StorageResult<()> {
        self.inner.make_directory(&self.full(prefix)?)
    }

    fn delete_directory(&self, prefix: &str) -> StorageResult<()> {
        self.inner.delete_directory(&self.full(prefix)?)
    }

    fn size(&self, key: &str) -> StorageResult<u64> {
        self.inner.size(&self.full(key)?)
    }

    fn last_modified(&self, key: &str) -> StorageResult<SystemTime> {
        self.inner.last_modified(&self.full(key)?)
    }

    fn mime_type(&self, key: &str) -> StorageResult<String> {
        self.inner.mime_type(&self.full(key)?)
    }

    fn path(&self, key: &str) -> StorageResult<PathBuf> {
        self.inner.path(&self.full(key)?)
    }

    fn visibility(&self, key: &str) -> StorageResult<Visibility> {
        self.inner.visibility(&self.full(key)?)
    }

    fn set_visibility(&self, key: &str, visibility: Visibility) -> StorageResult<()> {
        self.inner.set_visibility(&self.full(key)?, visibility)
    }

    fn default_visibility(&self) -> Visibility {
        self.inner.default_visibility()
    }

    fn temporary_upload_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        self.inner.temporary_upload_url(&self.full(key)?, ttl)
    }

    fn verify_temporary_upload_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        self.inner
            .verify_temporary_upload_url(&self.full(key)?, expires_at, signature)
    }
}

#[derive(Clone)]
pub struct ReadOnlyStorage {
    inner: Arc<dyn StorageDriver>,
}

impl ReadOnlyStorage {
    pub fn new(inner: Arc<dyn StorageDriver>) -> Self {
        Self { inner }
    }

    fn deny() -> StorageError {
        StorageError::ReadOnly
    }
}

impl StorageDriver for ReadOnlyStorage {
    fn put(&self, _key: &str, _bytes: &[u8]) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        self.inner.get(key)
    }

    fn delete(&self, _key: &str) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn url(&self, key: &str) -> String {
        self.inner.url(key)
    }

    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        self.inner.temporary_url(key, ttl)
    }

    fn verify_temporary_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        self.inner.verify_temporary_url(key, expires_at, signature)
    }

    fn exists(&self, key: &str) -> StorageResult<bool> {
        self.inner.exists(key)
    }

    fn copy(&self, _from: &str, _to: &str) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn rename(&self, _from: &str, _to: &str) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn prepend(&self, _key: &str, _bytes: &[u8]) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn append(&self, _key: &str, _bytes: &[u8]) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.files(prefix)
    }

    fn all_files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.all_files(prefix)
    }

    fn directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.directories(prefix)
    }

    fn all_directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.inner.all_directories(prefix)
    }

    fn make_directory(&self, _prefix: &str) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn delete_directory(&self, _prefix: &str) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn size(&self, key: &str) -> StorageResult<u64> {
        self.inner.size(key)
    }

    fn last_modified(&self, key: &str) -> StorageResult<SystemTime> {
        self.inner.last_modified(key)
    }

    fn mime_type(&self, key: &str) -> StorageResult<String> {
        self.inner.mime_type(key)
    }

    fn path(&self, key: &str) -> StorageResult<PathBuf> {
        self.inner.path(key)
    }

    fn visibility(&self, key: &str) -> StorageResult<Visibility> {
        self.inner.visibility(key)
    }

    fn set_visibility(&self, _key: &str, _visibility: Visibility) -> StorageResult<()> {
        Err(Self::deny())
    }

    fn default_visibility(&self) -> Visibility {
        self.inner.default_visibility()
    }

    fn temporary_upload_url(&self, _key: &str, _ttl: Duration) -> StorageResult<TemporaryUrl> {
        Err(Self::deny())
    }

    fn verify_temporary_upload_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        self.inner
            .verify_temporary_upload_url(key, expires_at, signature)
    }
}
