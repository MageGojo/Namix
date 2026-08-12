//! Cache abstraction with an in-memory driver and a Redis adapter.
//!
//! Cache failures are explicit. A backend outage and a missing key are
//! different states, so callers never have to infer an outage from `None`.

use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("cache value serialization failed")]
    Serialize(#[source] serde_json::Error),
    #[error("cache value for `{key}` is invalid")]
    Deserialize {
        key: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("cache backend `{operation}` failed: {message}")]
    Backend {
        operation: &'static str,
        message: String,
    },
    #[error("memory cache lock poisoned")]
    LockPoisoned,
}

pub type CacheResult<T> = Result<T, CacheError>;

impl From<CacheError> for crate::AppError {
    fn from(error: CacheError) -> Self {
        Self::internal(error)
    }
}

pub trait CacheStore: Send + Sync + 'static {
    fn get_raw(&self, key: &str) -> CacheResult<Option<Vec<u8>>>;
    fn put_raw(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> CacheResult<()>;
    fn forget(&self, key: &str) -> CacheResult<()>;
    fn flush(&self) -> CacheResult<()>;
}

#[derive(Clone)]
pub struct Cache {
    store: Arc<dyn CacheStore>,
}

impl Cache {
    pub fn new(store: impl CacheStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    pub fn get<T: DeserializeOwned>(&self, key: &str) -> CacheResult<Option<T>> {
        self.store
            .get_raw(key)?
            .map(|raw| {
                serde_json::from_slice(&raw).map_err(|source| CacheError::Deserialize {
                    key: key.to_owned(),
                    source,
                })
            })
            .transpose()
    }

    pub fn put<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl: Option<Duration>,
    ) -> CacheResult<()> {
        let value = serde_json::to_vec(value).map_err(CacheError::Serialize)?;
        self.store.put_raw(key, value, ttl)
    }

    pub fn remember<T: Serialize + DeserializeOwned>(
        &self,
        key: &str,
        ttl: Duration,
        load: impl FnOnce() -> T,
    ) -> CacheResult<T> {
        self.try_remember(key, ttl, || Ok(load()))
    }

    /// Return the cached value or run a fallible loader and cache its result.
    /// Loader and backend errors are returned without being collapsed into a
    /// cache miss.
    pub fn try_remember<T: Serialize + DeserializeOwned>(
        &self,
        key: &str,
        ttl: Duration,
        load: impl FnOnce() -> CacheResult<T>,
    ) -> CacheResult<T> {
        if let Some(value) = self.get(key)? {
            return Ok(value);
        }

        let value = load()?;
        self.put(key, &value, Some(ttl))?;
        Ok(value)
    }

    pub fn forget(&self, key: &str) -> CacheResult<()> {
        self.store.forget(key)
    }

    pub fn flush(&self) -> CacheResult<()> {
        self.store.flush()
    }
}

#[derive(Default, Clone)]
pub struct MemoryCache {
    values: Arc<RwLock<HashMap<String, Entry>>>,
}

struct Entry {
    value: Vec<u8>,
    expires_at: Option<SystemTime>,
}

impl CacheStore for MemoryCache {
    fn get_raw(&self, key: &str) -> CacheResult<Option<Vec<u8>>> {
        let mut values = self.values.write().map_err(|_| CacheError::LockPoisoned)?;
        let Some(entry) = values.get(key) else {
            return Ok(None);
        };
        if entry.expires_at.is_some_and(|at| at <= SystemTime::now()) {
            values.remove(key);
            Ok(None)
        } else {
            Ok(Some(entry.value.clone()))
        }
    }

    fn put_raw(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> CacheResult<()> {
        self.values
            .write()
            .map_err(|_| CacheError::LockPoisoned)?
            .insert(
                key.into(),
                Entry {
                    value,
                    expires_at: ttl.and_then(|ttl| SystemTime::now().checked_add(ttl)),
                },
            );
        Ok(())
    }

    fn forget(&self, key: &str) -> CacheResult<()> {
        self.values
            .write()
            .map_err(|_| CacheError::LockPoisoned)?
            .remove(key);
        Ok(())
    }

    fn flush(&self) -> CacheResult<()> {
        self.values
            .write()
            .map_err(|_| CacheError::LockPoisoned)?
            .clear();
        Ok(())
    }
}

/// Adapter contract shared by cache and session Redis integrations.
///
/// Existing application adapters return their provider message as a string;
/// [`RedisCache`] immediately wraps that message in a typed [`CacheError`]
/// before exposing it to cache callers.
pub trait RedisBackend: Send + Sync + 'static {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    fn set(&self, key: &str, value: &[u8], ttl: Option<Duration>) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
    fn flushdb(&self) -> Result<(), String>;
}

#[derive(Clone)]
pub struct RedisCache<B> {
    backend: Arc<B>,
}

impl<B> RedisCache<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend: Arc::new(backend),
        }
    }
}

impl<B: RedisBackend> CacheStore for RedisCache<B> {
    fn get_raw(&self, key: &str) -> CacheResult<Option<Vec<u8>>> {
        self.backend
            .get(key)
            .map_err(|message| CacheError::Backend {
                operation: "get",
                message,
            })
    }

    fn put_raw(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) -> CacheResult<()> {
        self.backend
            .set(key, &value, ttl)
            .map_err(|message| CacheError::Backend {
                operation: "set",
                message,
            })
    }

    fn forget(&self, key: &str) -> CacheResult<()> {
        self.backend
            .delete(key)
            .map_err(|message| CacheError::Backend {
                operation: "delete",
                message,
            })
    }

    fn flush(&self) -> CacheResult<()> {
        self.backend
            .flushdb()
            .map_err(|message| CacheError::Backend {
                operation: "flushdb",
                message,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_only_loads_once() {
        let cache = Cache::new(MemoryCache::default());
        let mut calls = 0;
        assert_eq!(
            cache
                .remember("x", Duration::from_secs(60), || {
                    calls += 1;
                    7
                })
                .unwrap(),
            7
        );
        assert_eq!(
            cache
                .remember("x", Duration::from_secs(60), || {
                    calls += 1;
                    9
                })
                .unwrap(),
            7
        );
        assert_eq!(calls, 1);
    }

    struct FailingRedis;

    impl RedisBackend for FailingRedis {
        fn get(&self, _key: &str) -> Result<Option<Vec<u8>>, String> {
            Err("connection reset".into())
        }

        fn set(&self, _key: &str, _value: &[u8], _ttl: Option<Duration>) -> Result<(), String> {
            Ok(())
        }

        fn delete(&self, _key: &str) -> Result<(), String> {
            Ok(())
        }

        fn flushdb(&self) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn backend_failure_is_not_a_cache_miss() {
        let cache = Cache::new(RedisCache::new(FailingRedis));
        let error = cache.get::<u64>("answer").unwrap_err();
        assert!(matches!(
            error,
            CacheError::Backend {
                operation: "get",
                ..
            }
        ));
        assert!(error.to_string().contains("connection reset"));
    }

    #[test]
    fn malformed_cached_value_is_typed() {
        let store = MemoryCache::default();
        store.put_raw("bad", b"{".to_vec(), None).unwrap();
        let error = Cache::new(store).get::<u64>("bad").unwrap_err();
        assert!(matches!(error, CacheError::Deserialize { .. }));
    }
}
