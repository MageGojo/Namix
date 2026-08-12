//! File-storage abstraction with local-disk and S3-compatible URL contracts.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use rand::RngCore as _;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const MIN_SIGNING_KEY_BYTES: usize = 32;
const TEMPORARY_URL_SIGNATURE_CONTEXT: &[u8] = b"namix-local-storage-url-v1\0";
const ATOMIC_WRITE_ATTEMPTS: usize = 32;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemporaryUrl {
    pub url: String,
    pub expires_at: u64,
}

/// Storage errors keep the operation and the underlying I/O source intact.
/// Callers can map invalid keys, signatures, and upload-policy variants to a
/// 4xx response, while I/O/backend/configuration failures become a logged
/// [`crate::AppError::internal`] error.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("invalid storage key")]
    InvalidKey,
    #[error("upload exceeds {max_bytes} bytes")]
    UploadTooLarge { max_bytes: usize },
    #[error("upload extension is not allowed")]
    ExtensionNotAllowed,
    #[error("temporary URL signature is invalid")]
    InvalidTemporaryUrlSignature,
    #[error("temporary URL expired at {expires_at}")]
    TemporaryUrlExpired { expires_at: u64 },
    #[error("storage signing key must contain at least {min_bytes} bytes")]
    SigningKeyTooShort { min_bytes: usize },
    #[error("temporary URL expiration exceeds the supported timestamp range")]
    ExpirationOverflow,
    #[error("this storage driver delegates temporary URL verification to its backend")]
    TemporaryUrlVerificationUnsupported,
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
            StorageError::InvalidTemporaryUrlSignature
            | StorageError::TemporaryUrlExpired { .. } => Self::Forbidden,
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

    /// Verify the `expires` and `signature` query values produced by
    /// [`StorageDriver::temporary_url`]. Drivers such as S3 that validate the
    /// signature at the object backend keep the default result.
    fn verify_temporary_url(
        &self,
        _key: &str,
        _expires_at: u64,
        _signature: &str,
    ) -> StorageResult<()> {
        Err(StorageError::TemporaryUrlVerificationUnsupported)
    }
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

    /// Verify a local temporary URL after the router has extracted its storage
    /// key and the `expires`/`signature` query values.
    pub fn verify_temporary_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        self.driver
            .verify_temporary_url(key, expires_at, signature)
    }
}

#[derive(Clone)]
pub struct LocalStorage {
    root: PathBuf,
    public_base: String,
    signing_key: Arc<[u8]>,
}

impl fmt::Debug for LocalStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalStorage")
            .field("root", &self.root)
            .field("public_base", &self.public_base)
            .field("signing_key", &"[REDACTED]")
            .finish()
    }
}

impl LocalStorage {
    /// Create a local driver with a process-local random signing key.
    ///
    /// This is convenient for development. The generated key changes whenever
    /// the process restarts, so production and multi-process deployments should
    /// use [`LocalStorage::with_signing_key`] with the same persistent secret on
    /// every instance. Use at least 32 random bytes from a secret manager; do
    /// not use an application name, password, or other human-readable value.
    pub fn new(root: impl Into<PathBuf>, public_base: impl Into<String>) -> Self {
        let mut signing_key = [0_u8; MIN_SIGNING_KEY_BYTES];
        OsRng.fill_bytes(&mut signing_key);
        Self::from_signing_key(root, public_base, &signing_key)
    }

    /// Create a local driver with an explicit temporary-URL signing key.
    ///
    /// All processes serving the same storage root must receive the same
    /// persistent secret. Key rotation invalidates URLs created with the old
    /// key. The key is redacted from `Debug` output.
    pub fn with_signing_key(
        root: impl Into<PathBuf>,
        public_base: impl Into<String>,
        signing_key: impl AsRef<[u8]>,
    ) -> StorageResult<Self> {
        let signing_key = signing_key.as_ref();
        if signing_key.len() < MIN_SIGNING_KEY_BYTES {
            return Err(StorageError::SigningKeyTooShort {
                min_bytes: MIN_SIGNING_KEY_BYTES,
            });
        }
        Ok(Self::from_signing_key(root, public_base, signing_key))
    }

    fn from_signing_key(
        root: impl Into<PathBuf>,
        public_base: impl Into<String>,
        signing_key: &[u8],
    ) -> Self {
        Self {
            root: root.into(),
            public_base: public_base.into().trim_end_matches('/').into(),
            signing_key: Arc::from(signing_key),
        }
    }

    fn relative_path(key: &str) -> StorageResult<PathBuf> {
        // Storage keys are portable URL-style paths, not platform-native paths.
        // A strict canonical spelling also prevents signature aliases such as
        // `a/../b`, `a//b`, or `./b`.
        if key.is_empty()
            || key.starts_with('/')
            || key.contains('\0')
            || key.contains('\\')
            || key
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | ".."))
        {
            return Err(StorageError::InvalidKey);
        }

        let path = Path::new(key);
        if path.is_absolute()
            || path.components().any(|component| {
                !matches!(component, Component::Normal(_))
                    || matches!(
                        component,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
            })
        {
            return Err(StorageError::InvalidKey);
        }
        Ok(path.to_path_buf())
    }

    /// Resolve the root itself, intentionally following a release-managed root
    /// symlink. Every component below this canonical root is checked separately
    /// and may not be a symlink.
    fn root_for_write(&self) -> StorageResult<PathBuf> {
        fs::create_dir_all(&self.root).map_err(StorageError::Io)?;
        self.canonical_existing_root()
    }

    fn root_if_exists(&self) -> StorageResult<Option<PathBuf>> {
        match fs::canonicalize(&self.root) {
            Ok(root) => {
                Self::ensure_directory(&root)?;
                Ok(Some(root))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(StorageError::Io(error)),
        }
    }

    fn canonical_existing_root(&self) -> StorageResult<PathBuf> {
        let root = fs::canonicalize(&self.root).map_err(StorageError::Io)?;
        Self::ensure_directory(&root)?;
        Ok(root)
    }

    fn ensure_directory(path: &Path) -> StorageResult<()> {
        let metadata = fs::metadata(path).map_err(StorageError::Io)?;
        if metadata.is_dir() {
            Ok(())
        } else {
            Err(StorageError::Io(io::Error::new(
                io::ErrorKind::NotADirectory,
                "storage path component is not a directory",
            )))
        }
    }

    fn checked_write_path(&self, key: &str) -> StorageResult<PathBuf> {
        let relative = Self::relative_path(key)?;
        let root = self.root_for_write()?;
        let mut components = relative
            .components()
            .map(|component| match component {
                Component::Normal(value) => Ok(value.to_os_string()),
                _ => Err(StorageError::InvalidKey),
            })
            .collect::<StorageResult<Vec<OsString>>>()?;
        let file_name = components.pop().ok_or(StorageError::InvalidKey)?;
        let mut parent = root;

        for component in components {
            parent.push(component);
            Self::ensure_safe_directory(&parent)?;
        }

        let target = parent.join(file_name);
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(StorageError::InvalidKey)
            }
            Ok(_) => Ok(target),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(target),
            Err(error) => Err(StorageError::Io(error)),
        }
    }

    fn ensure_safe_directory(path: &Path) -> StorageResult<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => Self::validate_descendant_directory(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::create_dir(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(StorageError::Io(error)),
                }
                let metadata = fs::symlink_metadata(path).map_err(StorageError::Io)?;
                Self::validate_descendant_directory(metadata)
            }
            Err(error) => Err(StorageError::Io(error)),
        }
    }

    fn validate_descendant_directory(metadata: fs::Metadata) -> StorageResult<()> {
        if metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidKey);
        }
        if !metadata.is_dir() {
            return Err(StorageError::Io(io::Error::new(
                io::ErrorKind::NotADirectory,
                "storage path component is not a directory",
            )));
        }
        Ok(())
    }

    fn checked_existing_path(&self, key: &str) -> StorageResult<Option<PathBuf>> {
        let relative = Self::relative_path(key)?;
        let Some(root) = self.root_if_exists()? else {
            return Ok(None);
        };
        let component_count = relative.components().count();
        let mut current = root;

        for (index, component) in relative.components().enumerate() {
            let Component::Normal(component) = component else {
                return Err(StorageError::InvalidKey);
            };
            current.push(component);
            let metadata = match fs::symlink_metadata(&current) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(StorageError::Io(error)),
            };
            if metadata.file_type().is_symlink() {
                return Err(StorageError::InvalidKey);
            }
            if index + 1 < component_count && !metadata.is_dir() {
                return Err(StorageError::Io(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "storage path component is not a directory",
                )));
            }
        }
        Ok(Some(current))
    }

    fn signature(&self, key: &str, expires_at: u64) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.signing_key)
            .expect("HMAC accepts signing keys of every length");
        mac.update(TEMPORARY_URL_SIGNATURE_CONTEXT);
        mac.update(&(key.len() as u64).to_be_bytes());
        mac.update(key.as_bytes());
        mac.update(&expires_at.to_be_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    /// Verify a temporary URL signature against the current system clock.
    ///
    /// `key` is the decoded route path. `expires_at` and `signature` are the
    /// corresponding query values. The HMAC check is constant-time, and the
    /// expiration value is covered by the signature.
    pub fn verify_temporary_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        let now = epoch_seconds(SystemTime::now())?;
        self.verify_temporary_url_at(key, expires_at, signature, now)
    }

    fn verify_temporary_url_at(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
        now: u64,
    ) -> StorageResult<()> {
        Self::relative_path(key)?;

        let supplied_signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| StorageError::InvalidTemporaryUrlSignature)?;
        let mut mac = HmacSha256::new_from_slice(&self.signing_key)
            .expect("HMAC accepts signing keys of every length");
        mac.update(TEMPORARY_URL_SIGNATURE_CONTEXT);
        mac.update(&(key.len() as u64).to_be_bytes());
        mac.update(key.as_bytes());
        mac.update(&expires_at.to_be_bytes());
        mac.verify_slice(&supplied_signature)
            .map_err(|_| StorageError::InvalidTemporaryUrlSignature)?;

        if now >= expires_at {
            return Err(StorageError::TemporaryUrlExpired { expires_at });
        }
        Ok(())
    }
}

impl StorageDriver for LocalStorage {
    fn put(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        let path = self.checked_write_path(key)?;
        atomic_write(&path, bytes)
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let Some(path) = self.checked_existing_path(key)? else {
            return Ok(None);
        };
        fs::read(path).map(Some).map_err(StorageError::Io)
    }

    fn delete(&self, key: &str) -> StorageResult<()> {
        let Some(path) = self.checked_existing_path(key)? else {
            return Ok(());
        };
        fs::remove_file(path).map_err(StorageError::Io)
    }

    fn url(&self, key: &str) -> String {
        format!("{}/{}", self.public_base, encode_url_path(key))
    }

    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        Self::relative_path(key)?;
        let now = epoch_seconds(SystemTime::now())?;
        let round_up = u64::from(ttl.subsec_nanos() != 0);
        let ttl_secs = ttl
            .as_secs()
            .checked_add(round_up)
            .ok_or(StorageError::ExpirationOverflow)?;
        let expires_at = now
            .checked_add(ttl_secs)
            .ok_or(StorageError::ExpirationOverflow)?;
        let signature = self.signature(key, expires_at);
        let url = self.url(key);
        let separator = if url.contains('?') { '&' } else { '?' };
        Ok(TemporaryUrl {
            url: format!("{url}{separator}expires={expires_at}&signature={signature}"),
            expires_at,
        })
    }

    fn verify_temporary_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        LocalStorage::verify_temporary_url(self, key, expires_at, signature)
    }
}

struct TemporaryFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> StorageResult<()> {
    let parent = path.parent().ok_or(StorageError::InvalidKey)?;
    let mut random = [0_u8; 16];

    for _ in 0..ATOMIC_WRITE_ATTEMPTS {
        OsRng.fill_bytes(&mut random);
        let temporary_path = parent.join(format!(
            ".namix-upload-{}-{}.tmp",
            std::process::id(),
            URL_SAFE_NO_PAD.encode(random)
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(StorageError::Io(error)),
        };
        let mut cleanup = TemporaryFileGuard::new(temporary_path.clone());
        file.write_all(bytes).map_err(StorageError::Io)?;
        file.sync_all().map_err(StorageError::Io)?;
        drop(file);
        fs::rename(&temporary_path, path).map_err(StorageError::Io)?;
        cleanup.disarm();
        return Ok(());
    }

    Err(StorageError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to allocate a unique atomic-write temporary file",
    )))
}

fn epoch_seconds(time: SystemTime) -> StorageResult<u64> {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(StorageError::Clock)
}

fn encode_url_path(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len());
    for byte in key.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            encoded.push(char::from(byte));
        } else {
            use fmt::Write as _;
            write!(encoded, "%{byte:02X}").expect("writing to a String cannot fail");
        }
    }
    encoded
}

/// S3-compatible adapter boundary. Applications supply a concrete transport
/// (AWS SDK, MinIO, R2, etc.) while controllers keep the same `Storage` API.
/// S3 presigned URLs remain backend-owned: the object service validates them,
/// so the local verification method returns `TemporaryUrlVerificationUnsupported`.
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

    fn test_directory(label: &str) -> PathBuf {
        let mut random = [0_u8; 12];
        OsRng.fill_bytes(&mut random);
        std::env::temp_dir().join(format!(
            "namix-storage-{label}-{}-{}",
            std::process::id(),
            URL_SAFE_NO_PAD.encode(random)
        ))
    }

    fn explicit_storage(root: &Path) -> LocalStorage {
        LocalStorage::with_signing_key(root, "/files", [7_u8; MIN_SIGNING_KEY_BYTES]).unwrap()
    }

    fn query_value<'a>(url: &'a str, name: &str) -> &'a str {
        url.split_once('?')
            .unwrap()
            .1
            .split('&')
            .find_map(|pair| pair.split_once('='))
            .filter(|(key, _)| *key == name)
            .map(|(_, value)| value)
            .unwrap()
    }

    #[test]
    fn local_storage_blocks_traversal_and_empty_keys() {
        let dir = test_directory("traversal");
        let storage = explicit_storage(&dir);
        for key in [
            "", ".", "./file", "a/.", "../x", "a/../x", "a//x", "/x", "a\\x",
        ] {
            assert!(
                matches!(storage.put(key, b"no"), Err(StorageError::InvalidKey)),
                "key {key:?} should be rejected"
            );
        }
        storage.put("a/b.txt", b"ok").unwrap();
        assert_eq!(storage.get("a/b.txt").unwrap(), Some(b"ok".to_vec()));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn local_storage_allows_trusted_root_symlink_but_blocks_descendant_symlinks() {
        use std::os::unix::fs::symlink;

        let sandbox = test_directory("symlink");
        let real_root = sandbox.join("real-root");
        let outside = sandbox.join("outside");
        let trusted_root = sandbox.join("release-storage");
        fs::create_dir_all(&real_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&real_root, &trusted_root).unwrap();
        symlink(&outside, real_root.join("untrusted")).unwrap();

        let storage = explicit_storage(&trusted_root);
        storage.put("safe/file.txt", b"inside").unwrap();
        assert_eq!(fs::read(real_root.join("safe/file.txt")).unwrap(), b"inside");
        assert!(matches!(
            storage.put("untrusted/file.txt", b"outside"),
            Err(StorageError::InvalidKey)
        ));
        assert!(matches!(
            storage.get("untrusted/file.txt"),
            Err(StorageError::InvalidKey)
        ));
        assert!(!outside.join("file.txt").exists());
        let _ = fs::remove_dir_all(sandbox);
    }

    #[test]
    fn atomic_write_replaces_complete_files_and_cleans_failed_temporary_files() {
        let dir = test_directory("atomic");
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("file.txt");
        atomic_write(&target, b"old").unwrap();
        atomic_write(&target, b"complete replacement").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"complete replacement");

        let invalid_target = dir.join("existing-directory");
        fs::create_dir(&invalid_target).unwrap();
        assert!(matches!(
            atomic_write(&invalid_target, b"never committed"),
            Err(StorageError::Io(_))
        ));
        let temporary_files = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| name.to_string_lossy().starts_with(".namix-upload-"))
            .collect::<Vec<_>>();
        assert!(temporary_files.is_empty(), "{temporary_files:?}");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn temporary_url_signature_survives_instances_and_rejects_tampering() {
        let dir = test_directory("signature");
        let first = explicit_storage(&dir);
        let second = explicit_storage(&dir);
        let temporary = first
            .temporary_url("reports/quarter 1.pdf", Duration::from_secs(60))
            .unwrap();
        let signature = query_value(&temporary.url, "signature");
        assert!(temporary.url.contains("quarter%201.pdf"));
        second
            .verify_temporary_url("reports/quarter 1.pdf", temporary.expires_at, signature)
            .unwrap();

        assert!(matches!(
            second.verify_temporary_url("reports/quarter 2.pdf", temporary.expires_at, signature),
            Err(StorageError::InvalidTemporaryUrlSignature)
        ));
        assert!(matches!(
            second.verify_temporary_url(
                "reports/quarter 1.pdf",
                temporary.expires_at + 1,
                signature
            ),
            Err(StorageError::InvalidTemporaryUrlSignature)
        ));
        let mut tampered = signature.as_bytes().to_vec();
        tampered[0] = if tampered[0] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered).unwrap();
        assert!(matches!(
            second.verify_temporary_url(
                "reports/quarter 1.pdf",
                temporary.expires_at,
                &tampered
            ),
            Err(StorageError::InvalidTemporaryUrlSignature)
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn temporary_url_expiration_is_signed_and_enforced() {
        let dir = test_directory("expiration");
        let storage = explicit_storage(&dir);
        let expires_at = 10_000;
        let signature = storage.signature("private/file.txt", expires_at);
        storage
            .verify_temporary_url_at("private/file.txt", expires_at, &signature, expires_at - 1)
            .unwrap();
        assert!(matches!(
            storage.verify_temporary_url_at(
                "private/file.txt",
                expires_at,
                &signature,
                expires_at
            ),
            Err(StorageError::TemporaryUrlExpired {
                expires_at: 10_000
            })
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn random_signing_keys_are_process_local_and_explicit_keys_are_validated() {
        let dir = test_directory("random-key");
        let first = LocalStorage::new(&dir, "/files");
        let second = LocalStorage::new(&dir, "/files");
        let expires_at = epoch_seconds(SystemTime::now()).unwrap() + 60;
        let signature = first.signature("file.txt", expires_at);
        assert!(matches!(
            second.verify_temporary_url("file.txt", expires_at, &signature),
            Err(StorageError::InvalidTemporaryUrlSignature)
        ));
        assert!(matches!(
            LocalStorage::with_signing_key(&dir, "/files", [1_u8; 31]),
            Err(StorageError::SigningKeyTooShort { min_bytes: 32 })
        ));
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
    fn upload_and_signature_errors_map_to_typed_app_errors() {
        let invalid: crate::AppError = StorageError::InvalidKey.into();
        assert_eq!(invalid.status().as_u16(), 400);
        let signature: crate::AppError = StorageError::InvalidTemporaryUrlSignature.into();
        assert_eq!(signature.status().as_u16(), 403);
        let io: crate::AppError =
            StorageError::Io(std::io::Error::other("disk unavailable")).into();
        assert_eq!(io.status().as_u16(), 500);
        assert!(std::error::Error::source(&io).is_some());
    }
}
