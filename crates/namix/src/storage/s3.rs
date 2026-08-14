//! S3-compatible adapter boundary. Applications supply a concrete transport.

use std::sync::Arc;
use std::time::Duration;

use super::{StorageDriver, StorageResult, TemporaryUrl, encode_url_path, normalize_key};

/// S3-compatible adapter boundary. Applications supply a concrete transport
/// (AWS SDK, MinIO, R2, etc.) while controllers keep the same `Storage` API.
/// S3 presigned URLs remain backend-owned: the object service validates them,
/// so the local verification method returns `TemporaryUrlVerificationUnsupported`.
pub trait S3Transport: Send + Sync + 'static {
    fn put_object(&self, bucket: &str, key: &str, bytes: &[u8]) -> StorageResult<()>;
    fn get_object(&self, bucket: &str, key: &str) -> StorageResult<Option<Vec<u8>>>;
    fn delete_object(&self, bucket: &str, key: &str) -> StorageResult<()>;
    fn presign_get(&self, bucket: &str, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl>;

    fn presign_put(
        &self,
        _bucket: &str,
        _key: &str,
        _ttl: Duration,
    ) -> StorageResult<TemporaryUrl> {
        Err(super::StorageError::unsupported("temporary_upload_url"))
    }
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
        let key = normalize_key(key)?;
        self.transport.put_object(&self.bucket, key, bytes)
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let key = normalize_key(key)?;
        self.transport.get_object(&self.bucket, key)
    }

    fn delete(&self, key: &str) -> StorageResult<()> {
        let key = normalize_key(key)?;
        self.transport.delete_object(&self.bucket, key)
    }

    fn url(&self, key: &str) -> String {
        format!(
            "{}/{}/{}",
            self.endpoint,
            self.bucket,
            encode_url_path(key.trim_start_matches('/'))
        )
    }

    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        let key = normalize_key(key)?;
        self.transport.presign_get(&self.bucket, key, ttl)
    }

    fn temporary_upload_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        let key = normalize_key(key)?;
        self.transport.presign_put(&self.bucket, key, ttl)
    }
}
