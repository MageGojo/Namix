//! File-storage abstraction: named disks, local/S3 drivers, fakes, and HTTP serving.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore as _;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use thiserror::Error;

use namix_http::{ContentType, UploadedFile};

mod disks;
mod http;
mod image;
mod local;
mod memory;
mod s3;
mod wrap;

pub use disks::{csrf_except_prefixes, extend, init, routes, serve_prefixes};
pub use image::StorageImage;
pub use local::LocalStorage;
pub use memory::MemoryStorage;
pub use s3::{S3CompatibleStorage, S3Transport};
pub use wrap::{ReadOnlyStorage, ScopedStorage};

pub(crate) const MIN_SIGNING_KEY_BYTES: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TemporaryUrl {
    pub url: String,
    pub expires_at: u64,
}

/// Public (world-readable) vs private object visibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Private,
    Public,
}

impl Visibility {
    pub fn parse(value: &str) -> StorageResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "private" => Ok(Self::Private),
            "public" => Ok(Self::Public),
            other => Err(StorageError::backend(format!(
                "unknown storage visibility `{other}`"
            ))),
        }
    }
}

/// Storage errors keep the operation and the underlying I/O source intact.
/// Callers can map invalid keys, signatures, and upload-policy variants to a
/// 4xx response, while I/O/backend/configuration failures become a logged
/// [`crate::AppError::internal`] error.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("invalid storage key")]
    InvalidKey,
    #[error("storage object not found")]
    NotFound,
    #[error("storage disk is read-only")]
    ReadOnly,
    #[error("storage driver does not support {operation}")]
    Unsupported { operation: &'static str },
    #[error("unknown storage disk `{name}`")]
    UnknownDisk { name: String },
    #[error("stored JSON is invalid")]
    InvalidJson(#[source] serde_json::Error),
    #[error("image processing failed: {message}")]
    Image { message: String },
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

    pub fn unsupported(operation: &'static str) -> Self {
        Self::Unsupported { operation }
    }
}

impl From<StorageError> for crate::AppError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::InvalidKey => Self::bad_request("invalid storage key"),
            StorageError::NotFound => Self::NotFound,
            StorageError::ReadOnly => Self::Forbidden,
            StorageError::UnknownDisk { name } => {
                Self::bad_request(format!("unknown storage disk `{name}`"))
            }
            StorageError::Unsupported { operation } => {
                Self::bad_request(format!("storage does not support {operation}"))
            }
            StorageError::InvalidJson(_) => Self::bad_request("stored JSON is invalid"),
            StorageError::Image { message } => Self::bad_request(message),
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

    fn exists(&self, key: &str) -> StorageResult<bool> {
        Ok(self.get(key)?.is_some())
    }

    fn copy(&self, from: &str, to: &str) -> StorageResult<()> {
        let bytes = self.get(from)?.ok_or(StorageError::NotFound)?;
        self.put(to, &bytes)
    }

    fn rename(&self, from: &str, to: &str) -> StorageResult<()> {
        self.copy(from, to)?;
        self.delete(from)
    }

    fn prepend(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        let mut out = bytes.to_vec();
        if let Some(existing) = self.get(key)? {
            out.extend_from_slice(&existing);
        }
        self.put(key, &out)
    }

    fn append(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        let mut out = self.get(key)?.unwrap_or_default();
        out.extend_from_slice(bytes);
        self.put(key, &out)
    }

    fn files(&self, _prefix: &str) -> StorageResult<Vec<String>> {
        Err(StorageError::unsupported("files"))
    }

    fn all_files(&self, _prefix: &str) -> StorageResult<Vec<String>> {
        Err(StorageError::unsupported("all_files"))
    }

    fn directories(&self, _prefix: &str) -> StorageResult<Vec<String>> {
        Err(StorageError::unsupported("directories"))
    }

    fn all_directories(&self, _prefix: &str) -> StorageResult<Vec<String>> {
        Err(StorageError::unsupported("all_directories"))
    }

    fn make_directory(&self, _prefix: &str) -> StorageResult<()> {
        Err(StorageError::unsupported("make_directory"))
    }

    fn delete_directory(&self, _prefix: &str) -> StorageResult<()> {
        Err(StorageError::unsupported("delete_directory"))
    }

    fn size(&self, key: &str) -> StorageResult<u64> {
        Ok(self.get(key)?.ok_or(StorageError::NotFound)?.len() as u64)
    }

    fn last_modified(&self, _key: &str) -> StorageResult<SystemTime> {
        Err(StorageError::unsupported("last_modified"))
    }

    fn mime_type(&self, key: &str) -> StorageResult<String> {
        normalize_key(key)?;
        Ok(ContentType::from_path(key).as_str().to_string())
    }

    fn path(&self, _key: &str) -> StorageResult<PathBuf> {
        Err(StorageError::unsupported("path"))
    }

    fn visibility(&self, _key: &str) -> StorageResult<Visibility> {
        Ok(self.default_visibility())
    }

    fn set_visibility(&self, _key: &str, _visibility: Visibility) -> StorageResult<()> {
        Err(StorageError::unsupported("set_visibility"))
    }

    fn default_visibility(&self) -> Visibility {
        Visibility::Private
    }

    fn temporary_upload_url(&self, _key: &str, _ttl: Duration) -> StorageResult<TemporaryUrl> {
        Err(StorageError::unsupported("temporary_upload_url"))
    }

    fn verify_temporary_upload_url(
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

impl fmt::Debug for Storage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Storage").finish_non_exhaustive()
    }
}

impl Storage {
    pub fn new(driver: impl StorageDriver) -> Self {
        Self {
            driver: Arc::new(driver),
        }
    }

    pub fn from_arc(driver: Arc<dyn StorageDriver>) -> Self {
        Self { driver }
    }

    pub(crate) fn driver_arc(&self) -> Arc<dyn StorageDriver> {
        Arc::clone(&self.driver)
    }

    /// Named disk installed by Boot / [`Storage::fake`].
    pub fn disk(name: &str) -> StorageResult<Self> {
        disks::disk(name)
    }

    /// Default disk from `[storage].default`.
    pub fn default_disk() -> StorageResult<Self> {
        disks::default_disk()
    }

    /// Swap a disk for an in-memory fake (tests).
    pub fn fake(name: impl Into<String>) -> Self {
        disks::fake(name.into())
    }

    pub fn extend(
        driver: impl Into<String>,
        factory: impl Fn(&crate::config::DiskConfig) -> StorageResult<Arc<dyn StorageDriver>>
        + Send
        + Sync
        + 'static,
    ) {
        disks::extend(driver, factory);
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

    pub fn put_file(&self, dir: &str, file: &UploadedFile) -> StorageResult<String> {
        let name = random_object_name(file.extension());
        self.put_file_as(dir, file, &name)
    }

    pub fn put_file_as(&self, dir: &str, file: &UploadedFile, name: &str) -> StorageResult<String> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains('\0')
            || name == "."
            || name == ".."
        {
            return Err(StorageError::InvalidKey);
        }
        let key = if dir.is_empty() {
            name.to_string()
        } else {
            format!("{}/{name}", dir.trim_end_matches('/'))
        };
        self.put(&key, &file.data)?;
        Ok(key)
    }

    pub fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        self.driver.get(key)
    }

    pub fn json<T: DeserializeOwned>(&self, key: &str) -> StorageResult<T> {
        let bytes = self.get(key)?.ok_or(StorageError::NotFound)?;
        serde_json::from_slice(&bytes).map_err(StorageError::InvalidJson)
    }

    pub fn put_json<T: Serialize>(&self, key: &str, value: &T) -> StorageResult<()> {
        let bytes = serde_json::to_vec(value).map_err(StorageError::InvalidJson)?;
        self.put(key, &bytes)
    }

    pub fn delete(&self, key: &str) -> StorageResult<()> {
        self.driver.delete(key)
    }

    pub fn exists(&self, key: &str) -> StorageResult<bool> {
        self.driver.exists(key)
    }

    pub fn missing(&self, key: &str) -> StorageResult<bool> {
        Ok(!self.exists(key)?)
    }

    pub fn copy(&self, from: &str, to: &str) -> StorageResult<()> {
        self.driver.copy(from, to)
    }

    pub fn rename(&self, from: &str, to: &str) -> StorageResult<()> {
        self.driver.rename(from, to)
    }

    pub fn prepend(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        self.driver.prepend(key, bytes)
    }

    pub fn append(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        self.driver.append(key, bytes)
    }

    pub fn files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.driver.files(prefix)
    }

    pub fn all_files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.driver.all_files(prefix)
    }

    pub fn directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.driver.directories(prefix)
    }

    pub fn all_directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.driver.all_directories(prefix)
    }

    pub fn make_directory(&self, prefix: &str) -> StorageResult<()> {
        self.driver.make_directory(prefix)
    }

    pub fn delete_directory(&self, prefix: &str) -> StorageResult<()> {
        self.driver.delete_directory(prefix)
    }

    pub fn size(&self, key: &str) -> StorageResult<u64> {
        self.driver.size(key)
    }

    pub fn last_modified(&self, key: &str) -> StorageResult<SystemTime> {
        self.driver.last_modified(key)
    }

    pub fn mime_type(&self, key: &str) -> StorageResult<String> {
        self.driver.mime_type(key)
    }

    pub fn path(&self, key: &str) -> StorageResult<PathBuf> {
        self.driver.path(key)
    }

    pub fn visibility(&self, key: &str) -> StorageResult<Visibility> {
        self.driver.visibility(key)
    }

    pub fn set_visibility(&self, key: &str, visibility: Visibility) -> StorageResult<()> {
        self.driver.set_visibility(key, visibility)
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
        self.driver.verify_temporary_url(key, expires_at, signature)
    }

    pub fn temporary_upload_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        self.driver.temporary_upload_url(key, ttl)
    }

    pub fn verify_temporary_upload_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        self.driver
            .verify_temporary_upload_url(key, expires_at, signature)
    }

    pub fn scoped(&self, prefix: &str) -> StorageResult<Self> {
        Ok(Self::new(ScopedStorage::new(
            Arc::clone(&self.driver),
            prefix,
        )?))
    }

    pub fn read_only(&self) -> Self {
        Self::new(ReadOnlyStorage::new(Arc::clone(&self.driver)))
    }

    pub fn image(&self, key: &str) -> StorageResult<StorageImage> {
        StorageImage::load(self.clone(), key)
    }

    pub fn assert_exists(&self, key: &str) {
        assert!(
            self.exists(key).expect("storage exists"),
            "storage key `{key}` does not exist"
        );
    }

    pub fn assert_missing(&self, key: &str) {
        assert!(
            self.missing(key).expect("storage missing"),
            "storage key `{key}` exists"
        );
    }
}

pub(crate) fn normalize_key(key: &str) -> StorageResult<&str> {
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
    Ok(key)
}

pub(crate) fn encode_url_path(key: &str) -> String {
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

pub(crate) fn epoch_seconds(time: SystemTime) -> StorageResult<u64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(StorageError::Clock)
}

fn random_object_name(extension: &str) -> String {
    let mut random = [0_u8; 16];
    OsRng.fill_bytes(&mut random);
    let stem = URL_SAFE_NO_PAD.encode(random);
    let extension = extension.trim().trim_start_matches('.');
    if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use namix_http::UploadedFile;

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
        let missing: crate::AppError = StorageError::NotFound.into();
        assert_eq!(missing.status().as_u16(), 404);
        let io: crate::AppError =
            StorageError::Io(std::io::Error::other("disk unavailable")).into();
        assert_eq!(io.status().as_u16(), 500);
        assert!(std::error::Error::source(&io).is_some());
    }

    #[test]
    fn fake_disk_supports_copy_json_and_put_file() {
        let photos = Storage::fake("photos");
        photos.put("a.txt", b"one").unwrap();
        photos.copy("a.txt", "b.txt").unwrap();
        photos.assert_exists("b.txt");
        photos.append("b.txt", b"-two").unwrap();
        assert_eq!(photos.get("b.txt").unwrap(), Some(b"one-two".to_vec()));
        photos
            .put_json("meta.json", &serde_json::json!({"ok": true}))
            .unwrap();
        let value: serde_json::Value = photos.json("meta.json").unwrap();
        assert_eq!(value["ok"], true);

        let file = UploadedFile {
            name: "avatar".into(),
            filename: "me.png".into(),
            content_type: "image/png".into(),
            data: bytes::Bytes::from_static(b"png-bytes"),
        };
        let key = photos.put_file("avatars", &file).unwrap();
        assert!(key.starts_with("avatars/"));
        assert!(key.ends_with(".png"));
        photos.assert_exists(&key);
        photos.delete(&key).unwrap();
        photos.assert_missing(&key);
    }

    #[test]
    fn scoped_and_read_only_wrap_a_disk() {
        let root = Storage::fake("wrap-root");
        root.put("avatars/a.png", b"img").unwrap();
        let avatars = root.scoped("avatars").unwrap();
        assert_eq!(avatars.get("a.png").unwrap(), Some(b"img".to_vec()));
        avatars.put("b.png", b"two").unwrap();
        root.assert_exists("avatars/b.png");

        let frozen = avatars.read_only();
        assert!(matches!(
            frozen.put("c.png", b"no"),
            Err(StorageError::ReadOnly)
        ));
        assert_eq!(frozen.get("a.png").unwrap(), Some(b"img".to_vec()));
    }
}
