//! In-memory disk used by [`crate::Storage::fake`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use super::{
    StorageDriver, StorageError, StorageResult, TemporaryUrl, Visibility, encode_url_path,
    normalize_key,
};

#[derive(Clone)]
struct Object {
    bytes: Vec<u8>,
    visibility: Visibility,
    modified: SystemTime,
}

#[derive(Default)]
struct Inner {
    objects: HashMap<String, Object>,
    directories: HashSet<String>,
}

#[derive(Clone)]
pub struct MemoryStorage {
    name: String,
    inner: Arc<Mutex<Inner>>,
    visibility: Visibility,
}

impl MemoryStorage {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            inner: Arc::new(Mutex::new(Inner::default())),
            visibility: Visibility::Public,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn child_of(prefix: &str, key: &str) -> bool {
        if prefix.is_empty() {
            return true;
        }
        key == prefix || key.starts_with(&format!("{prefix}/"))
    }

    fn direct_child(prefix: &str, key: &str) -> bool {
        let rest = if prefix.is_empty() {
            key
        } else {
            let Some(stripped) = key.strip_prefix(prefix) else {
                return false;
            };
            stripped.strip_prefix('/').unwrap_or(stripped)
        };
        !rest.is_empty() && !rest.contains('/')
    }
}

impl StorageDriver for MemoryStorage {
    fn put(&self, key: &str, bytes: &[u8]) -> StorageResult<()> {
        let key = normalize_key(key)?.to_string();
        let mut inner = self.lock();
        if let Some(parent) = key.rsplit_once('/').map(|(dir, _)| dir.to_string()) {
            inner.directories.insert(parent);
        }
        inner.objects.insert(
            key,
            Object {
                bytes: bytes.to_vec(),
                visibility: self.visibility,
                modified: SystemTime::now(),
            },
        );
        Ok(())
    }

    fn get(&self, key: &str) -> StorageResult<Option<Vec<u8>>> {
        let key = normalize_key(key)?;
        Ok(self
            .lock()
            .objects
            .get(key)
            .map(|object| object.bytes.clone()))
    }

    fn delete(&self, key: &str) -> StorageResult<()> {
        let key = normalize_key(key)?;
        self.lock().objects.remove(key);
        Ok(())
    }

    fn url(&self, key: &str) -> String {
        format!("/storage/fake/{}/{}", self.name, encode_url_path(key))
    }

    fn temporary_url(&self, key: &str, ttl: Duration) -> StorageResult<TemporaryUrl> {
        normalize_key(key)?;
        let expires_at = super::epoch_seconds(SystemTime::now())?
            .checked_add(ttl.as_secs().max(1))
            .ok_or(StorageError::ExpirationOverflow)?;
        Ok(TemporaryUrl {
            url: format!("{}?expires={expires_at}&signature=fake", self.url(key)),
            expires_at,
        })
    }

    fn exists(&self, key: &str) -> StorageResult<bool> {
        let key = normalize_key(key)?;
        Ok(self.lock().objects.contains_key(key))
    }

    fn files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        if !prefix.is_empty() {
            normalize_key(prefix)?;
        }
        let inner = self.lock();
        let mut keys: Vec<String> = inner
            .objects
            .keys()
            .filter(|key| Self::child_of(prefix, key) && Self::direct_child(prefix, key))
            .cloned()
            .collect();
        keys.sort();
        Ok(keys)
    }

    fn all_files(&self, prefix: &str) -> StorageResult<Vec<String>> {
        if !prefix.is_empty() {
            normalize_key(prefix)?;
        }
        let inner = self.lock();
        let mut keys: Vec<String> = inner
            .objects
            .keys()
            .filter(|key| Self::child_of(prefix, key) && *key != prefix)
            .cloned()
            .collect();
        keys.sort();
        Ok(keys)
    }

    fn directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        if !prefix.is_empty() {
            normalize_key(prefix)?;
        }
        let inner = self.lock();
        let mut dirs: HashSet<String> = inner.directories.clone();
        for key in inner.objects.keys() {
            if let Some((dir, _)) = key.rsplit_once('/') {
                dirs.insert(dir.to_string());
            }
        }
        let mut keys: Vec<String> = dirs
            .into_iter()
            .filter(|dir| Self::child_of(prefix, dir) && Self::direct_child(prefix, dir))
            .collect();
        keys.sort();
        Ok(keys)
    }

    fn all_directories(&self, prefix: &str) -> StorageResult<Vec<String>> {
        if !prefix.is_empty() {
            normalize_key(prefix)?;
        }
        let inner = self.lock();
        let mut dirs: HashSet<String> = inner.directories.clone();
        for key in inner.objects.keys() {
            if let Some((dir, _)) = key.rsplit_once('/') {
                dirs.insert(dir.to_string());
            }
        }
        let mut keys: Vec<String> = dirs
            .into_iter()
            .filter(|dir| Self::child_of(prefix, dir) && dir.as_str() != prefix)
            .collect();
        keys.sort();
        Ok(keys)
    }

    fn make_directory(&self, prefix: &str) -> StorageResult<()> {
        let prefix = normalize_key(prefix)?.to_string();
        self.lock().directories.insert(prefix);
        Ok(())
    }

    fn delete_directory(&self, prefix: &str) -> StorageResult<()> {
        let prefix = normalize_key(prefix)?;
        let mut inner = self.lock();
        inner.objects.retain(|key, _| !Self::child_of(prefix, key));
        inner.directories.retain(|dir| !Self::child_of(prefix, dir));
        Ok(())
    }

    fn size(&self, key: &str) -> StorageResult<u64> {
        let key = normalize_key(key)?;
        self.lock()
            .objects
            .get(key)
            .map(|object| object.bytes.len() as u64)
            .ok_or(StorageError::NotFound)
    }

    fn last_modified(&self, key: &str) -> StorageResult<SystemTime> {
        let key = normalize_key(key)?;
        self.lock()
            .objects
            .get(key)
            .map(|object| object.modified)
            .ok_or(StorageError::NotFound)
    }

    fn path(&self, key: &str) -> StorageResult<PathBuf> {
        let key = normalize_key(key)?;
        Ok(PathBuf::from(format!("memory:{}/{key}", self.name)))
    }

    fn visibility(&self, key: &str) -> StorageResult<Visibility> {
        let key = normalize_key(key)?;
        self.lock()
            .objects
            .get(key)
            .map(|object| object.visibility)
            .ok_or(StorageError::NotFound)
    }

    fn set_visibility(&self, key: &str, visibility: Visibility) -> StorageResult<()> {
        let key = normalize_key(key)?;
        let mut inner = self.lock();
        let object = inner.objects.get_mut(key).ok_or(StorageError::NotFound)?;
        object.visibility = visibility;
        Ok(())
    }

    fn default_visibility(&self) -> Visibility {
        self.visibility
    }
}
