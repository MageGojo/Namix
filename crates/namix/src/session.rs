//! Session store abstraction.
//!
//! - [`MemorySessionStore`]: default for single-process development.
//! - [`FileSessionStore`]: shared via `dist/data/storage` (symlink), enabling
//!   overlap-safe rolling updates on one host without Redis.
//! - [`RedisSessionStore`]: adapter over [`crate::RedisBackend`].
//!
//! Cookie signing stays at the application edge; this module only persists the
//! authenticated session payload keyed by an opaque session id.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

use crate::cache::RedisBackend;

/// Authenticated session payload stored under an opaque id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthSession {
    pub user_id: u64,
    pub username: String,
    pub is_vip: bool,
    #[serde(default = "default_session_role")]
    pub role: String,
    #[serde(default)]
    pub email_verified: bool,
    /// Absolute expiry as Unix seconds.
    pub expires_at_unix: u64,
}

fn default_session_role() -> String {
    "user".into()
}

impl AuthSession {
    pub fn expires_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.expires_at_unix)
    }

    pub fn is_expired(&self) -> bool {
        SystemTime::now() >= self.expires_at()
    }

    pub fn with_ttl(
        user_id: u64,
        username: impl Into<String>,
        is_vip: bool,
        ttl: Duration,
    ) -> Self {
        let expires_at_unix = SystemTime::now()
            .checked_add(ttl)
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        Self {
            user_id,
            username: username.into(),
            is_vip,
            role: "user".into(),
            email_verified: false,
            expires_at_unix,
        }
    }

    pub fn with_actor(
        user_id: u64,
        username: impl Into<String>,
        is_vip: bool,
        role: impl Into<String>,
        email_verified: bool,
        ttl: Duration,
    ) -> Self {
        let mut session = Self::with_ttl(user_id, username, is_vip, ttl);
        session.role = role.into();
        if session.role.trim().is_empty() {
            session.role = "user".into();
        }
        session.email_verified = email_verified;
        session
    }

    pub fn role(&self) -> &str {
        if self.role.trim().is_empty() {
            "user"
        } else {
            self.role.as_str()
        }
    }
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session store I/O failed")]
    Io(#[source] std::io::Error),
    #[error("session payload encode/decode failed")]
    Codec(#[source] serde_json::Error),
    #[error("session backend failed: {message}")]
    Backend { message: String },
}

pub type SessionResult<T> = Result<T, SessionError>;

impl From<SessionError> for crate::AppError {
    fn from(error: SessionError) -> Self {
        Self::internal(error)
    }
}

impl SessionError {
    pub fn backend(message: impl Into<String>) -> Self {
        Self::Backend {
            message: message.into(),
        }
    }
}

/// Persistence backend for authenticated sessions.
pub trait SessionStore: Send + Sync + 'static {
    fn get(&self, id: &str) -> SessionResult<Option<AuthSession>>;
    fn put(&self, id: &str, session: &AuthSession) -> SessionResult<()>;
    fn forget(&self, id: &str) -> SessionResult<()>;
    fn forget_user(&self, user_id: u64) -> SessionResult<usize>;
    /// Shared stores survive process overlap during rolling updates.
    fn is_shared(&self) -> bool;
}

/// Facade used by application session services.
#[derive(Clone)]
pub struct Session {
    store: Arc<dyn SessionStore>,
}

impl Session {
    pub fn new(store: impl SessionStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    pub fn from_arc(store: Arc<dyn SessionStore>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Arc<dyn SessionStore> {
        &self.store
    }

    pub fn is_shared(&self) -> bool {
        self.store.is_shared()
    }

    pub fn get(&self, id: &str) -> SessionResult<Option<AuthSession>> {
        match self.store.get(id)? {
            Some(session) if session.is_expired() => {
                let _ = self.store.forget(id);
                Ok(None)
            }
            other => Ok(other),
        }
    }

    pub fn put(&self, id: &str, session: &AuthSession) -> SessionResult<()> {
        self.store.put(id, session)
    }

    pub fn forget(&self, id: &str) -> SessionResult<()> {
        self.store.forget(id)
    }

    pub fn forget_user(&self, user_id: u64) -> SessionResult<usize> {
        self.store.forget_user(user_id)
    }
}

static INSTALLED: RwLock<Option<Session>> = RwLock::new(None);

/// Install the process-wide session facade (called by [`crate::Boot`]).
///
/// Replacing the store is allowed so tests can isolate drivers; production
/// boots once during startup.
pub fn install(session: Session) {
    *INSTALLED.write().expect("session install lock") = Some(session);
}

/// Process-wide session facade. Defaults to a process-wide in-memory store
/// until [`install`] replaces it (Boot does this from `[session]`).
pub fn current() -> Session {
    if let Some(session) = INSTALLED.read().expect("session install lock").clone() {
        return session;
    }
    let mut guard = INSTALLED.write().expect("session install lock");
    if let Some(session) = guard.clone() {
        return session;
    }
    let session = Session::new(MemorySessionStore::default());
    *guard = Some(session.clone());
    session
}

/// Whether a configured driver name is safe across overlapping processes.
pub fn driver_is_shared(driver: &str) -> bool {
    matches!(
        driver.trim().to_ascii_lowercase().as_str(),
        "file" | "redis"
    )
}

/// Build a store from `[session]` configuration.
pub fn store_from_config(section: &crate::config::SessionSection) -> SessionResult<Session> {
    let driver = section.driver.trim().to_ascii_lowercase();
    let session = match driver.as_str() {
        "memory" | "" => Session::new(MemorySessionStore::default()),
        "file" => Session::new(FileSessionStore::open(&section.path)?),
        "redis" => {
            // Applications wire a concrete Redis client and install it before
            // Boot. Config only declares the intent + validates sharing.
            let existing = INSTALLED
                .read()
                .expect("session install lock")
                .clone()
                .filter(|session| session.is_shared());
            if let Some(session) = existing {
                return Ok(session);
            }
            return Err(SessionError::backend(
                "session.driver=redis requires an application-provided RedisBackend; \
                 call namix::session::install(Session::new(RedisSessionStore::new(backend))) \
                 before Boot::run, or use driver=file for shared single-host releases",
            ));
        }
        "database" | "db" => {
            return Err(SessionError::backend(
                "session.driver=database is reserved; use driver=file (shared data plane) \
                 or wire RedisSessionStore until a DB driver ships",
            ));
        }
        other => {
            return Err(SessionError::backend(format!(
                "unsupported session.driver={other:?}; expected memory, file, or redis"
            )));
        }
    };
    Ok(session)
}

#[derive(Default)]
pub struct MemorySessionStore {
    values: RwLock<HashMap<String, AuthSession>>,
}

impl SessionStore for MemorySessionStore {
    fn get(&self, id: &str) -> SessionResult<Option<AuthSession>> {
        Ok(self.values.read().expect("session memory").get(id).cloned())
    }

    fn put(&self, id: &str, session: &AuthSession) -> SessionResult<()> {
        self.values
            .write()
            .expect("session memory")
            .insert(id.to_string(), session.clone());
        Ok(())
    }

    fn forget(&self, id: &str) -> SessionResult<()> {
        self.values.write().expect("session memory").remove(id);
        Ok(())
    }

    fn forget_user(&self, user_id: u64) -> SessionResult<usize> {
        let mut guard = self.values.write().expect("session memory");
        let before = guard.len();
        guard.retain(|_, session| session.user_id != user_id);
        Ok(before - guard.len())
    }

    fn is_shared(&self) -> bool {
        false
    }
}

/// JSON files under a shared directory (typically `./storage/sessions`).
///
/// In releases, `storage` is a symlink into `dist/data/storage`, so candidate
/// and draining processes see the same session records.
pub struct FileSessionStore {
    root: PathBuf,
}

impl FileSessionStore {
    pub fn open(path: impl AsRef<Path>) -> SessionResult<Self> {
        let root = path.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(SessionError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
                .map_err(SessionError::Io)?;
        }
        Ok(Self { root })
    }

    fn path_for(&self, id: &str) -> SessionResult<PathBuf> {
        if id.is_empty()
            || id.contains('/')
            || id.contains('\\')
            || id.contains("..")
            || id
                .chars()
                .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        {
            return Err(SessionError::backend("invalid session id for file store"));
        }
        Ok(self.root.join(format!("{id}.json")))
    }

    fn read_file(&self, path: &Path) -> SessionResult<Option<AuthSession>> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(SessionError::backend("session path is not a regular file"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(SessionError::Io(error)),
        }
        match fs::read(path) {
            Ok(bytes) => {
                let session: AuthSession =
                    serde_json::from_slice(&bytes).map_err(SessionError::Codec)?;
                Ok(Some(session))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(SessionError::Io(error)),
        }
    }
}

impl SessionStore for FileSessionStore {
    fn get(&self, id: &str) -> SessionResult<Option<AuthSession>> {
        let path = self.path_for(id)?;
        self.read_file(&path)
    }

    fn put(&self, id: &str, session: &AuthSession) -> SessionResult<()> {
        let path = self.path_for(id)?;
        let bytes = serde_json::to_vec(session).map_err(SessionError::Codec)?;
        static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
        let tmp = self.root.join(format!(
            ".{id}.{}.{}.tmp",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| -> SessionResult<()> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp).map_err(SessionError::Io)?;
            file.write_all(&bytes).map_err(SessionError::Io)?;
            file.sync_all().map_err(SessionError::Io)?;
            fs::rename(&tmp, &path).map_err(SessionError::Io)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result?;
        Ok(())
    }

    fn forget(&self, id: &str) -> SessionResult<()> {
        let path = self.path_for(id)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SessionError::Io(error)),
        }
    }

    fn forget_user(&self, user_id: u64) -> SessionResult<usize> {
        let mut removed = 0usize;
        let entries = fs::read_dir(&self.root).map_err(SessionError::Io)?;
        for entry in entries {
            let entry = entry.map_err(SessionError::Io)?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Some(session) = self.read_file(&path)?
                && session.user_id == user_id
            {
                match fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(SessionError::Io(error)),
                }
            }
        }
        Ok(removed)
    }

    fn is_shared(&self) -> bool {
        true
    }
}

/// Redis-backed sessions via the small [`RedisBackend`] contract.
pub struct RedisSessionStore<B> {
    backend: Arc<B>,
    prefix: String,
}

impl<B> RedisSessionStore<B> {
    pub fn new(backend: B) -> Self {
        Self::with_prefix(backend, "namix:session:")
    }

    pub fn with_prefix(backend: B, prefix: impl Into<String>) -> Self {
        Self {
            backend: Arc::new(backend),
            prefix: prefix.into(),
        }
    }

    fn key(&self, id: &str) -> String {
        format!("{}{id}", self.prefix)
    }

    fn user_key(&self, user_id: u64) -> String {
        format!("{}user:{user_id}", self.prefix)
    }

    fn ttl_remaining(session: &AuthSession) -> Option<Duration> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        session
            .expires_at_unix
            .checked_sub(now)
            .filter(|&secs| secs > 0)
            .map(Duration::from_secs)
    }
}

impl<B: RedisBackend> SessionStore for RedisSessionStore<B> {
    fn get(&self, id: &str) -> SessionResult<Option<AuthSession>> {
        let raw = self
            .backend
            .get(&self.key(id))
            .map_err(SessionError::backend)?;
        match raw {
            Some(bytes) => {
                let session: AuthSession =
                    serde_json::from_slice(&bytes).map_err(SessionError::Codec)?;
                Ok(Some(session))
            }
            None => Ok(None),
        }
    }

    fn put(&self, id: &str, session: &AuthSession) -> SessionResult<()> {
        let bytes = serde_json::to_vec(session).map_err(SessionError::Codec)?;
        let ttl = Self::ttl_remaining(session);

        // Keep a coarse per-user index so revoke-all can fan out without SCAN.
        // Publish the index before the credential: a failed second write can
        // leave a harmless stale id, while the reverse order could leave a
        // live session that revoke-all never sees.
        let user_key = self.user_key(session.user_id);
        let mut ids: Vec<String> =
            match self.backend.get(&user_key).map_err(SessionError::backend)? {
                Some(raw) => serde_json::from_slice(&raw).map_err(SessionError::Codec)?,
                None => Vec::new(),
            };
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
        let index = serde_json::to_vec(&ids).map_err(SessionError::Codec)?;
        self.backend
            .set(&user_key, &index, ttl)
            .map_err(SessionError::backend)?;
        self.backend
            .set(&self.key(id), &bytes, ttl)
            .map_err(SessionError::backend)?;
        Ok(())
    }

    fn forget(&self, id: &str) -> SessionResult<()> {
        let session = self.get(id)?;
        // Revoke the credential first. A stale user index is cleanup debt;
        // leaving the credential alive because index maintenance failed would
        // make logout appear successful while the token still works.
        self.backend
            .delete(&self.key(id))
            .map_err(SessionError::backend)?;
        if let Some(session) = session {
            let user_key = self.user_key(session.user_id);
            if let Some(raw) = self.backend.get(&user_key).map_err(SessionError::backend)? {
                let mut ids: Vec<String> =
                    serde_json::from_slice(&raw).map_err(SessionError::Codec)?;
                ids.retain(|existing| existing != id);
                if ids.is_empty() {
                    self.backend
                        .delete(&user_key)
                        .map_err(SessionError::backend)?;
                } else {
                    let index = serde_json::to_vec(&ids).map_err(SessionError::Codec)?;
                    self.backend
                        .set(&user_key, &index, Self::ttl_remaining(&session))
                        .map_err(SessionError::backend)?;
                }
            }
        }
        Ok(())
    }

    fn forget_user(&self, user_id: u64) -> SessionResult<usize> {
        let user_key = self.user_key(user_id);
        let ids: Vec<String> = match self.backend.get(&user_key).map_err(SessionError::backend)? {
            Some(raw) => serde_json::from_slice(&raw).map_err(SessionError::Codec)?,
            None => Vec::new(),
        };
        let mut removed = 0usize;
        for id in &ids {
            self.backend
                .delete(&self.key(id))
                .map_err(SessionError::backend)?;
            removed += 1;
        }
        self.backend
            .delete(&user_key)
            .map_err(SessionError::backend)?;
        Ok(removed)
    }

    fn is_shared(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn memory_store_round_trips_and_marks_unshared() {
        let store = MemorySessionStore::default();
        let session = AuthSession::with_ttl(7, "alice", true, Duration::from_secs(60));
        store.put("abc", &session).unwrap();
        assert_eq!(store.get("abc").unwrap().unwrap().username, "alice");
        assert!(!store.is_shared());
        assert_eq!(store.forget_user(7).unwrap(), 1);
        assert!(store.get("abc").unwrap().is_none());
    }

    #[test]
    fn file_store_is_shared_across_handles() {
        let dir = std::env::temp_dir().join(format!(
            "namix-session-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let a = FileSessionStore::open(&dir).unwrap();
        let b = FileSessionStore::open(&dir).unwrap();
        let session = AuthSession::with_ttl(1, "bob", false, Duration::from_secs(30));
        a.put("sid1", &session).unwrap();
        assert!(b.is_shared());
        assert_eq!(b.get("sid1").unwrap().unwrap().user_id, 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(dir.join("sid1.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert_eq!(b.forget_user(1).unwrap(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn session_facade_drops_expired_records() {
        let session_api = Session::new(MemorySessionStore::default());
        let expired = AuthSession::with_ttl(3, "eve", false, Duration::ZERO);
        session_api.put("gone", &expired).unwrap();
        assert!(session_api.get("gone").unwrap().is_none());
    }

    struct MapRedis {
        values: Mutex<HashMap<String, Vec<u8>>>,
    }

    impl RedisBackend for MapRedis {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &[u8], _ttl: Option<Duration>) -> Result<(), String> {
            self.values
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_vec());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
        fn flushdb(&self) -> Result<(), String> {
            self.values.lock().unwrap().clear();
            Ok(())
        }
    }

    #[test]
    fn redis_adapter_indexes_users_for_revoke_all() {
        let backend = MapRedis {
            values: Mutex::new(HashMap::new()),
        };
        let store = RedisSessionStore::new(backend);
        let session = AuthSession::with_ttl(9, "zoe", false, Duration::from_secs(120));
        store.put("one", &session).unwrap();
        store.put("two", &session).unwrap();
        assert_eq!(store.forget_user(9).unwrap(), 2);
        assert!(store.get("one").unwrap().is_none());
    }

    #[test]
    fn redis_adapter_does_not_silently_replace_a_corrupt_user_index() {
        let mut values = HashMap::new();
        values.insert("namix:session:user:9".into(), b"not-json".to_vec());
        let backend = MapRedis {
            values: Mutex::new(values),
        };
        let store = RedisSessionStore::new(backend);
        let session = AuthSession::with_ttl(9, "zoe", false, Duration::from_secs(120));
        assert!(matches!(
            store.put("one", &session),
            Err(SessionError::Codec(_))
        ));
    }

    #[test]
    fn driver_sharing_matrix() {
        assert!(!driver_is_shared("memory"));
        assert!(driver_is_shared("file"));
        assert!(driver_is_shared("redis"));
        assert!(!driver_is_shared("database"));
    }
}
