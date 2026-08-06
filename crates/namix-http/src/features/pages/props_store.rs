//! 一次性 props 暂存：HTML 只带 key，客户端 `GET /__namix/props/:key` 领取后即删。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

const TTL: Duration = Duration::from_secs(60);

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

static SEQ: AtomicU64 = AtomicU64::new(1);

pub fn put(component: String, props: Value, url: String) -> String {
    let key = next_key();
    let mut map = store().lock().unwrap_or_else(|e| e.into_inner());
    purge_expired(&mut map);
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
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // 不可猜测的短 key（进程内一次性）
    format!("{t:x}-{n:x}-{:x}", std::process::id())
}
