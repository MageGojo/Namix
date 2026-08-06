//! Cache abstraction with an in-memory driver.

use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

pub trait CacheStore: Send + Sync + 'static {
    fn get_raw(&self, key: &str) -> Option<Vec<u8>>;
    fn put_raw(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>);
    fn forget(&self, key: &str);
    fn flush(&self);
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
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.store
            .get_raw(key)
            .and_then(|raw| serde_json::from_slice(&raw).ok())
    }
    pub fn put<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        ttl: Option<Duration>,
    ) -> Result<(), serde_json::Error> {
        self.store.put_raw(key, serde_json::to_vec(value)?, ttl);
        Ok(())
    }
    pub fn remember<T: Serialize + DeserializeOwned>(
        &self,
        key: &str,
        ttl: Duration,
        load: impl FnOnce() -> T,
    ) -> T {
        self.get(key).unwrap_or_else(|| {
            let value = load();
            let _ = self.put(key, &value, Some(ttl));
            value
        })
    }
    pub fn forget(&self, key: &str) {
        self.store.forget(key)
    }
    pub fn flush(&self) {
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
    fn get_raw(&self, key: &str) -> Option<Vec<u8>> {
        let mut values = self.values.write().expect("cache state");
        let entry = values.get(key)?;
        if entry.expires_at.is_some_and(|at| at <= SystemTime::now()) {
            values.remove(key);
            None
        } else {
            Some(entry.value.clone())
        }
    }
    fn put_raw(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) {
        self.values.write().expect("cache state").insert(
            key.into(),
            Entry {
                value,
                expires_at: ttl.and_then(|ttl| SystemTime::now().checked_add(ttl)),
            },
        );
    }
    fn forget(&self, key: &str) {
        self.values.write().expect("cache state").remove(key);
    }
    fn flush(&self) {
        self.values.write().expect("cache state").clear();
    }
}

/// Adapter contract for Redis clients.  Keeping the client behind this small
/// trait lets applications choose `redis`, deadpool, or a managed provider
/// without changing controller cache calls.
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
    fn get_raw(&self, key: &str) -> Option<Vec<u8>> {
        self.backend.get(key).ok().flatten()
    }
    fn put_raw(&self, key: &str, value: Vec<u8>, ttl: Option<Duration>) {
        let _ = self.backend.set(key, &value, ttl);
    }
    fn forget(&self, key: &str) {
        let _ = self.backend.delete(key);
    }
    fn flush(&self) {
        let _ = self.backend.flushdb();
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
            cache.remember("x", Duration::from_secs(60), || {
                calls += 1;
                7
            }),
            7
        );
        assert_eq!(
            cache.remember("x", Duration::from_secs(60), || {
                calls += 1;
                9
            }),
            7
        );
        assert_eq!(calls, 1);
    }
}
