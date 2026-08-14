//! Local disk driver: traversal-safe, atomic writes, HMAC temporary URLs.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use rand::RngCore as _;
use rand::rngs::OsRng;
use sha2::Sha256;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::{
    MIN_SIGNING_KEY_BYTES, StorageDriver, StorageError, StorageResult, TemporaryUrl, Visibility,
    encode_url_path, epoch_seconds, normalize_key,
};

const TEMPORARY_URL_SIGNATURE_CONTEXT: &[u8] = b"namix-local-storage-url-v1\0";
const TEMPORARY_UPLOAD_SIGNATURE_CONTEXT: &[u8] = b"namix-local-storage-upload-v1\0";
const ATOMIC_WRITE_ATTEMPTS: usize = 32;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct LocalStorage {
    root: PathBuf,
    public_base: String,
    signing_key: Arc<[u8]>,
    visibility: Visibility,
}

impl fmt::Debug for LocalStorage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalStorage")
            .field("root", &self.root)
            .field("public_base", &self.public_base)
            .field("visibility", &self.visibility)
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
        Self::from_signing_key(root, public_base, &signing_key, Visibility::Private)
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
        Ok(Self::from_signing_key(
            root,
            public_base,
            signing_key,
            Visibility::Private,
        ))
    }

    pub fn with_visibility(self, visibility: Visibility) -> Self {
        Self { visibility, ..self }
    }

    fn from_signing_key(
        root: impl Into<PathBuf>,
        public_base: impl Into<String>,
        signing_key: &[u8],
        visibility: Visibility,
    ) -> Self {
        Self {
            root: root.into(),
            public_base: public_base.into().trim_end_matches('/').into(),
            signing_key: Arc::from(signing_key),
            visibility,
        }
    }

    fn relative_path(key: &str) -> StorageResult<PathBuf> {
        let key = normalize_key(key)?;
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

    fn relative_prefix(prefix: &str) -> StorageResult<PathBuf> {
        if prefix.is_empty() {
            return Ok(PathBuf::new());
        }
        Self::relative_path(prefix)
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
            Ok(metadata) if metadata.file_type().is_symlink() => Err(StorageError::InvalidKey),
            Ok(_) => Ok(target),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(target),
            Err(error) => Err(StorageError::Io(error)),
        }
    }

    fn checked_directory_path(&self, key: &str) -> StorageResult<PathBuf> {
        let relative = Self::relative_path(key)?;
        let mut current = self.root_for_write()?;
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(StorageError::InvalidKey);
            };
            current.push(name);
            Self::ensure_safe_directory(&current)?;
        }
        Ok(current)
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

    fn key_from_path(root: &Path, path: &Path) -> StorageResult<String> {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| StorageError::InvalidKey)?;
        let mut key = String::new();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(StorageError::InvalidKey);
            };
            let name = name.to_str().ok_or(StorageError::InvalidKey)?;
            if !key.is_empty() {
                key.push('/');
            }
            key.push_str(name);
        }
        Ok(key)
    }

    fn list(
        &self,
        prefix: &str,
        recursive: bool,
        want_files: bool,
        want_dirs: bool,
    ) -> StorageResult<Vec<String>> {
        Self::relative_prefix(prefix)?;
        let Some(root) = self.root_if_exists()? else {
            return Ok(Vec::new());
        };
        let start = if prefix.is_empty() {
            root.clone()
        } else {
            match self.checked_existing_path(prefix)? {
                Some(path) => path,
                None => return Ok(Vec::new()),
            }
        };
        let metadata = match fs::symlink_metadata(&start) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(StorageError::Io(error)),
        };
        if metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidKey);
        }
        if !metadata.is_dir() {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        self.walk(&root, &start, recursive, want_files, want_dirs, &mut out)?;
        out.sort();
        Ok(out)
    }

    fn walk(
        &self,
        root: &Path,
        dir: &Path,
        recursive: bool,
        want_files: bool,
        want_dirs: bool,
        out: &mut Vec<String>,
    ) -> StorageResult<()> {
        let entries = fs::read_dir(dir).map_err(StorageError::Io)?;
        for entry in entries {
            let entry = entry.map_err(StorageError::Io)?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(StorageError::Io)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            let key = Self::key_from_path(root, &path)?;
            if metadata.is_dir() {
                if want_dirs {
                    out.push(key);
                }
                if recursive {
                    self.walk(root, &path, recursive, want_files, want_dirs, out)?;
                }
            } else if metadata.is_file()
                && want_files
                && !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".namix-upload-")
            {
                out.push(key);
            }
        }
        Ok(())
    }

    fn delete_tree(path: &Path) -> StorageResult<()> {
        let metadata = fs::symlink_metadata(path).map_err(StorageError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidKey);
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path).map_err(StorageError::Io)? {
                Self::delete_tree(&entry.map_err(StorageError::Io)?.path())?;
            }
            fs::remove_dir(path).map_err(StorageError::Io)
        } else {
            fs::remove_file(path).map_err(StorageError::Io)
        }
    }

    fn apply_visibility(&self, path: &Path, visibility: Visibility) -> StorageResult<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = match visibility {
                Visibility::Public => 0o644,
                Visibility::Private => 0o600,
            };
            fs::set_permissions(path, fs::Permissions::from_mode(mode))
                .map_err(StorageError::Io)?;
        }
        #[cfg(not(unix))]
        {
            let _ = (path, visibility);
        }
        Ok(())
    }

    fn read_visibility(&self, path: &Path) -> StorageResult<Visibility> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(path)
                .map_err(StorageError::Io)?
                .permissions()
                .mode()
                & 0o777;
            if mode & 0o004 != 0 {
                Ok(Visibility::Public)
            } else {
                Ok(Visibility::Private)
            }
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Ok(self.visibility)
        }
    }

    fn signature_with(&self, context: &[u8], key: &str, expires_at: u64) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.signing_key)
            .expect("HMAC accepts signing keys of every length");
        mac.update(context);
        mac.update(&(key.len() as u64).to_be_bytes());
        mac.update(key.as_bytes());
        mac.update(&expires_at.to_be_bytes());
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }

    fn signed_url(&self, context: &[u8], key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        normalize_key(key)?;
        let now = epoch_seconds(SystemTime::now())?;
        let round_up = u64::from(ttl.subsec_nanos() != 0);
        let ttl_secs = ttl
            .as_secs()
            .checked_add(round_up)
            .ok_or(StorageError::ExpirationOverflow)?;
        let expires_at = now
            .checked_add(ttl_secs)
            .ok_or(StorageError::ExpirationOverflow)?;
        let signature = self.signature_with(context, key, expires_at);
        let url = self.url(key);
        let separator = if url.contains('?') { '&' } else { '?' };
        Ok(TemporaryUrl {
            url: format!("{url}{separator}expires={expires_at}&signature={signature}"),
            expires_at,
        })
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
        self.verify_at(
            TEMPORARY_URL_SIGNATURE_CONTEXT,
            key,
            expires_at,
            signature,
            now,
        )
    }

    fn verify_at(
        &self,
        context: &[u8],
        key: &str,
        expires_at: u64,
        signature: &str,
        now: u64,
    ) -> StorageResult<()> {
        normalize_key(key)?;

        let supplied_signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| StorageError::InvalidTemporaryUrlSignature)?;
        let mut mac = HmacSha256::new_from_slice(&self.signing_key)
            .expect("HMAC accepts signing keys of every length");
        mac.update(context);
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
        atomic_write(&path, bytes)?;
        self.apply_visibility(&path, self.visibility)
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let Some(path) = self.checked_existing_path(key)? else {
            return Ok(None);
        };
        let metadata = fs::symlink_metadata(&path).map_err(StorageError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidKey);
        }
        if !metadata.is_file() {
            return Ok(None);
        }
        fs::read(path).map(Some).map_err(StorageError::Io)
    }

    fn delete(&self, key: &str) -> StorageResult<()> {
        let Some(path) = self.checked_existing_path(key)? else {
            return Ok(());
        };
        let metadata = fs::symlink_metadata(&path).map_err(StorageError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidKey);
        }
        if !metadata.is_file() {
            return Ok(());
        }
        fs::remove_file(path).map_err(StorageError::Io)
    }

    fn url(&self, key: &str) -> String {
        if self.public_base.is_empty() {
            encode_url_path(key)
        } else {
            format!("{}/{}", self.public_base, encode_url_path(key))
        }
    }

    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        self.signed_url(TEMPORARY_URL_SIGNATURE_CONTEXT, key, ttl)
    }

    fn verify_temporary_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        LocalStorage::verify_temporary_url(self, key, expires_at, signature)
    }

    fn exists(&self, key: &str) -> StorageResult<bool> {
        let Some(path) = self.checked_existing_path(key)? else {
            return Ok(false);
        };
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(StorageError::Io(error)),
        };
        if metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidKey);
        }
        Ok(metadata.is_file())
    }

    fn files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.list(prefix, false, true, false)
    }

    fn all_files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.list(prefix, true, true, false)
    }

    fn directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.list(prefix, false, false, true)
    }

    fn all_directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        self.list(prefix, true, false, true)
    }

    fn make_directory(&self, prefix: &str) -> StorageResult<()> {
        self.checked_directory_path(prefix)?;
        Ok(())
    }

    fn delete_directory(&self, prefix: &str) -> StorageResult<()> {
        let Some(path) = self.checked_existing_path(prefix)? else {
            return Ok(());
        };
        let metadata = fs::symlink_metadata(&path).map_err(StorageError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(StorageError::InvalidKey);
        }
        if !metadata.is_dir() {
            return Ok(());
        }
        Self::delete_tree(&path)
    }

    fn size(&self, key: &str) -> StorageResult<u64> {
        let path = self
            .checked_existing_path(key)?
            .ok_or(StorageError::NotFound)?;
        let metadata = fs::symlink_metadata(&path).map_err(StorageError::Io)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StorageError::NotFound);
        }
        Ok(metadata.len())
    }

    fn last_modified(&self, key: &str) -> StorageResult<SystemTime> {
        let path = self
            .checked_existing_path(key)?
            .ok_or(StorageError::NotFound)?;
        fs::metadata(path)
            .map_err(StorageError::Io)?
            .modified()
            .map_err(StorageError::Io)
    }

    fn path(&self, key: &str) -> StorageResult<PathBuf> {
        let relative = Self::relative_path(key)?;
        Ok(self.root.join(relative))
    }

    fn visibility(&self, key: &str) -> StorageResult<Visibility> {
        let path = self
            .checked_existing_path(key)?
            .ok_or(StorageError::NotFound)?;
        self.read_visibility(&path)
    }

    fn set_visibility(&self, key: &str, visibility: Visibility) -> StorageResult<()> {
        let path = self
            .checked_existing_path(key)?
            .ok_or(StorageError::NotFound)?;
        self.apply_visibility(&path, visibility)
    }

    fn default_visibility(&self) -> Visibility {
        self.visibility
    }

    fn temporary_upload_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        self.signed_url(TEMPORARY_UPLOAD_SIGNATURE_CONTEXT, key, ttl)
    }

    fn verify_temporary_upload_url(
        &self,
        key: &str,
        expires_at: u64,
        signature: &str,
    ) -> StorageResult<()> {
        let now = epoch_seconds(SystemTime::now())?;
        self.verify_at(
            TEMPORARY_UPLOAD_SIGNATURE_CONTEXT,
            key,
            expires_at,
            signature,
            now,
        )
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

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> StorageResult<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MIN_SIGNING_KEY_BYTES;

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
            .filter_map(|pair| pair.split_once('='))
            .find(|(key, _)| *key == name)
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
        assert_eq!(
            fs::read(real_root.join("safe/file.txt")).unwrap(),
            b"inside"
        );
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
            second.verify_temporary_url("reports/quarter 1.pdf", temporary.expires_at, &tampered),
            Err(StorageError::InvalidTemporaryUrlSignature)
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn temporary_url_expiration_is_signed_and_enforced() {
        let dir = test_directory("expiration");
        let storage = explicit_storage(&dir);
        let expires_at = 10_000;
        let signature = storage.signature_with(
            TEMPORARY_URL_SIGNATURE_CONTEXT,
            "private/file.txt",
            expires_at,
        );
        storage
            .verify_at(
                TEMPORARY_URL_SIGNATURE_CONTEXT,
                "private/file.txt",
                expires_at,
                &signature,
                expires_at - 1,
            )
            .unwrap();
        assert!(matches!(
            storage.verify_at(
                TEMPORARY_URL_SIGNATURE_CONTEXT,
                "private/file.txt",
                expires_at,
                &signature,
                expires_at
            ),
            Err(StorageError::TemporaryUrlExpired { expires_at: 10_000 })
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn random_signing_keys_are_process_local_and_explicit_keys_are_validated() {
        let dir = test_directory("random-key");
        let first = LocalStorage::new(&dir, "/files");
        let second = LocalStorage::new(&dir, "/files");
        let expires_at = epoch_seconds(SystemTime::now()).unwrap() + 60;
        let signature =
            first.signature_with(TEMPORARY_URL_SIGNATURE_CONTEXT, "file.txt", expires_at);
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
    fn local_lists_files_and_directories() {
        let dir = test_directory("list");
        let storage = explicit_storage(&dir);
        storage.put("avatars/a.png", b"a").unwrap();
        storage.put("avatars/2024/b.png", b"b").unwrap();
        storage.put("readme.txt", b"hi").unwrap();
        storage.make_directory("empty-dir").unwrap();

        assert_eq!(storage.files("").unwrap(), vec!["readme.txt"]);
        assert_eq!(storage.files("avatars").unwrap(), vec!["avatars/a.png"]);
        assert_eq!(
            storage.all_files("avatars").unwrap(),
            vec!["avatars/2024/b.png", "avatars/a.png"]
        );
        assert!(
            storage
                .directories("")
                .unwrap()
                .iter()
                .any(|name| name == "avatars" || name == "empty-dir")
        );
        storage.delete_directory("avatars").unwrap();
        assert!(!storage.exists("avatars/a.png").unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn download_signature_cannot_be_reused_for_upload() {
        let dir = test_directory("upload-sig");
        let storage = explicit_storage(&dir);
        let download = storage
            .temporary_url("reports/a.txt", Duration::from_secs(60))
            .unwrap();
        let signature = query_value(&download.url, "signature");
        assert!(matches!(
            storage.verify_temporary_upload_url("reports/a.txt", download.expires_at, signature),
            Err(StorageError::InvalidTemporaryUrlSignature)
        ));
        let upload = storage
            .temporary_upload_url("reports/a.txt", Duration::from_secs(60))
            .unwrap();
        storage
            .verify_temporary_upload_url(
                "reports/a.txt",
                upload.expires_at,
                query_value(&upload.url, "signature"),
            )
            .unwrap();
        let _ = fs::remove_dir_all(dir);
    }
}
