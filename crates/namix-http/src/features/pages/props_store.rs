//! 一次性 props 暂存：HTML 只带 key，客户端 `GET /__namix/props/:key` 领取后即删。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rand::RngCore as _;
use serde_json::Value;

const TTL: Duration = Duration::from_secs(60);
const MAX_ENTRIES: usize = 4_096;

pub struct Entry {
    pub component: String,
    pub props: Value,
    pub url: String,
    expires: Instant,
}

fn store() -> &'static Mutex<HashMap<String, Entry>> {
    static S: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn put(component: String, props: Value, url: String) -> String {
    let mut map = store().lock().unwrap_or_else(|e| e.into_inner());
    purge_expired(&mut map);
    if map.len() >= MAX_ENTRIES
        && let Some(oldest) = map
            .iter()
            .min_by_key(|(_, entry)| entry.expires)
            .map(|(key, _)| key.clone())
    {
        map.remove(&oldest);
    }
    let key = loop {
        let candidate = next_key();
        if !map.contains_key(&candidate) {
            break candidate;
        }
    };
    map.insert(
        key.clone(),
        Entry {
            component,
            props,
            url,
            expires: Instant::now() + TTL,
        },
    );
    key
}

pub fn take(key: &str) -> Option<Entry> {
    if key.is_empty() {
        return None;
    }
    let mut map = store().lock().unwrap_or_else(|e| e.into_inner());
    purge_expired(&mut map);
    let entry = map.remove(key)?;
    if entry.expires <= Instant::now() {
        return None;
    }
    Some(entry)
}

fn purge_expired(map: &mut HashMap<String, Entry>) {
    let now = Instant::now();
    map.retain(|_, e| e.expires > now);
}

fn next_key() -> String {
    let mut random = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut random);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_random_url_safe_and_one_time() {
        let first = put("home".into(), Value::Null, "/".into());
        let second = put("home".into(), Value::Null, "/".into());
        assert_ne!(first, second);
        assert_eq!(first.len(), 32);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        );
        assert!(take(&first).is_some());
        assert!(take(&first).is_none());
        let _ = take(&second);
    }
}
