//! Named disks from `[storage]` plus `Storage::extend` / `Storage::fake`.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::config::{DiskConfig, StorageSection};

use super::local::LocalStorage;
use super::memory::MemoryStorage;
use super::wrap::{ReadOnlyStorage, ScopedStorage};
use super::{
    MIN_SIGNING_KEY_BYTES, Storage, StorageDriver, StorageError, StorageResult, Visibility,
};

struct InstalledDisk {
    storage: Storage,
    url_base: String,
    visibility: Visibility,
}

impl InstalledDisk {
    fn driver_arc(&self) -> Arc<dyn StorageDriver> {
        self.storage.driver_arc()
    }
}

struct Manager {
    default: String,
    disks: HashMap<String, InstalledDisk>,
}

type DriverFactory =
    Arc<dyn Fn(&DiskConfig) -> StorageResult<Arc<dyn StorageDriver>> + Send + Sync>;

static MANAGER: RwLock<Option<Manager>> = RwLock::new(None);
static EXTENSIONS: RwLock<Option<HashMap<String, DriverFactory>>> = RwLock::new(None);

pub fn extend(
    driver: impl Into<String>,
    factory: impl Fn(&DiskConfig) -> StorageResult<Arc<dyn StorageDriver>> + Send + Sync + 'static,
) {
    let mut extensions = EXTENSIONS.write().expect("storage extend lock");
    let map = extensions.get_or_insert_with(HashMap::new);
    map.insert(driver.into(), Arc::new(factory));
}

pub fn init(cfg: &StorageSection) -> StorageResult<()> {
    let signing_key = signing_key();
    let disks_cfg = resolved_disks(cfg);
    let default = if cfg.default.trim().is_empty() {
        "local".to_string()
    } else {
        cfg.default.trim().to_string()
    };

    let mut concrete: HashMap<String, InstalledDisk> = HashMap::new();
    let mut deferred = Vec::new();

    for (name, disk) in &disks_cfg {
        let driver = disk.driver.trim().to_ascii_lowercase();
        if matches!(driver.as_str(), "scoped" | "readonly" | "read-only") {
            deferred.push((name.clone(), disk.clone()));
            continue;
        }
        let installed = build_concrete(name, disk, &signing_key)?;
        concrete.insert(name.clone(), installed);
    }

    for (name, disk) in deferred {
        let source_name = disk.disk.trim();
        if source_name.is_empty() {
            return Err(StorageError::backend(format!(
                "storage disk `{name}` ({}) requires `disk = \"...\"`",
                disk.driver
            )));
        }
        let source = concrete.get(source_name).ok_or_else(|| {
            StorageError::backend(format!(
                "storage disk `{name}` references unknown disk `{source_name}`"
            ))
        })?;
        let driver = disk.driver.trim().to_ascii_lowercase();
        let wrapped = if driver == "scoped" {
            let prefix = disk.prefix.trim();
            if prefix.is_empty() {
                return Err(StorageError::backend(format!(
                    "storage disk `{name}` (scoped) requires `prefix`"
                )));
            }
            Storage::new(ScopedStorage::new(source.driver_arc(), prefix)?)
        } else {
            Storage::new(ReadOnlyStorage::new(source.driver_arc()))
        };
        let url_base = if disk.url.trim().is_empty() {
            source.url_base.clone()
        } else {
            normalize_url_base(&disk.url)
        };
        let visibility = if disk.visibility.trim().is_empty() {
            source.visibility
        } else {
            Visibility::parse(&disk.visibility)?
        };
        concrete.insert(
            name,
            InstalledDisk {
                storage: wrapped,
                url_base,
                visibility,
            },
        );
    }

    if !concrete.contains_key(&default) {
        return Err(StorageError::UnknownDisk { name: default });
    }

    let names: Vec<&str> = concrete.keys().map(String::as_str).collect();
    crate::log::info!("storage → default={default} disks={}", names.join(","));

    let mut manager = MANAGER.write().expect("storage manager lock");
    *manager = Some(Manager {
        default,
        disks: concrete,
    });
    Ok(())
}

pub fn disk(name: &str) -> StorageResult<Storage> {
    let manager = MANAGER.read().expect("storage manager lock");
    manager
        .as_ref()
        .and_then(|manager| manager.disks.get(name).map(|disk| disk.storage.clone()))
        .ok_or_else(|| StorageError::UnknownDisk {
            name: name.to_string(),
        })
}

pub fn default_disk() -> StorageResult<Storage> {
    let manager = MANAGER.read().expect("storage manager lock");
    let Some(manager) = manager.as_ref() else {
        return Err(StorageError::UnknownDisk {
            name: "default".into(),
        });
    };
    manager
        .disks
        .get(&manager.default)
        .map(|disk| disk.storage.clone())
        .ok_or_else(|| StorageError::UnknownDisk {
            name: manager.default.clone(),
        })
}

pub fn fake(name: String) -> Storage {
    let storage = Storage::new(MemoryStorage::new(&name));
    let installed = InstalledDisk {
        storage: storage.clone(),
        url_base: format!("/storage/fake/{name}"),
        visibility: Visibility::Public,
    };
    let mut manager = MANAGER.write().expect("storage manager lock");
    match manager.as_mut() {
        Some(manager) => {
            manager.disks.insert(name, installed);
        }
        None => {
            *manager = Some(Manager {
                default: name.clone(),
                disks: HashMap::from([(name, installed)]),
            });
        }
    }
    storage
}

pub fn serve_prefixes() -> Vec<String> {
    let manager = MANAGER.read().expect("storage manager lock");
    let Some(manager) = manager.as_ref() else {
        return Vec::new();
    };
    let mut prefixes: Vec<String> = manager
        .disks
        .values()
        .map(|disk| disk.url_base.clone())
        .filter(|base| !base.is_empty())
        .collect();
    prefixes.sort();
    prefixes.dedup();
    prefixes
}

pub fn csrf_except_prefixes() -> Vec<String> {
    serve_prefixes()
}

pub fn routes() -> namix_http::Router {
    super::http::routes()
}

pub(crate) fn disk_for_url(path: &str) -> Option<(String, InstalledMeta)> {
    let manager = MANAGER.read().expect("storage manager lock");
    let manager = manager.as_ref()?;
    let mut best: Option<(usize, String, InstalledMeta)> = None;
    for (name, disk) in &manager.disks {
        if disk.url_base.is_empty() {
            continue;
        }
        if path == disk.url_base || path.starts_with(&format!("{}/", disk.url_base)) {
            let len = disk.url_base.len();
            if best.as_ref().is_none_or(|(best_len, _, _)| len > *best_len) {
                best = Some((
                    len,
                    name.clone(),
                    InstalledMeta {
                        storage: disk.storage.clone(),
                        visibility: disk.visibility,
                    },
                ));
            }
        }
    }
    best.map(|(_, name, meta)| (name, meta))
}

#[derive(Clone)]
pub(crate) struct InstalledMeta {
    pub storage: Storage,
    pub visibility: Visibility,
}

fn resolved_disks(cfg: &StorageSection) -> HashMap<String, DiskConfig> {
    if cfg.disks.is_empty() {
        return default_disks();
    }
    cfg.disks
        .iter()
        .map(|(name, disk)| (name.clone(), disk.clone()))
        .collect()
}

fn default_disks() -> HashMap<String, DiskConfig> {
    HashMap::from([
        (
            "local".into(),
            DiskConfig {
                driver: "local".into(),
                root: "./storage/app".into(),
                url: "/storage/private".into(),
                visibility: "private".into(),
                ..DiskConfig::default()
            },
        ),
        (
            "public".into(),
            DiskConfig {
                driver: "local".into(),
                root: "./storage/app/public".into(),
                url: "/storage".into(),
                visibility: "public".into(),
                ..DiskConfig::default()
            },
        ),
    ])
}

fn build_concrete(
    name: &str,
    disk: &DiskConfig,
    signing_key: &[u8],
) -> StorageResult<InstalledDisk> {
    let driver = disk.driver.trim().to_ascii_lowercase();
    let visibility = Visibility::parse(&disk.visibility)?;
    let url_base = normalize_url_base(&disk.url);
    match driver.as_str() {
        "local" => {
            let root = if disk.root.trim().is_empty() {
                if name == "public" {
                    "./storage/app/public".to_string()
                } else {
                    "./storage/app".to_string()
                }
            } else {
                disk.root.trim().to_string()
            };
            let local = LocalStorage::with_signing_key(root, url_base.clone(), signing_key)?
                .with_visibility(visibility);
            Ok(InstalledDisk {
                storage: Storage::new(local),
                url_base,
                visibility,
            })
        }
        other => {
            let extensions = EXTENSIONS.read().expect("storage extend lock");
            let Some(factory) = extensions.as_ref().and_then(|map| map.get(other)) else {
                return Err(StorageError::backend(format!(
                    "storage disk `{name}` uses driver `{other}`; register it with Storage::extend(\"{other}\", …) \
                     (FTP/SFTP/S3 are not built-in — Laravel also installs a separate package)"
                )));
            };
            let driver = factory(disk)?;
            Ok(InstalledDisk {
                storage: Storage::from_arc(driver),
                url_base,
                visibility,
            })
        }
    }
}

fn normalize_url_base(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    if url.is_empty() {
        String::new()
    } else if url.starts_with('/') {
        url.to_string()
    } else {
        format!("/{url}")
    }
}

fn signing_key() -> [u8; MIN_SIGNING_KEY_BYTES] {
    let secret = crate::config::session_secret()
        .map(str::to_string)
        .or_else(|| std::env::var("NAMIX_SESSION_SECRET").ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("dev-storage-{}", std::process::id()));
    let mut hasher = Sha256::new();
    hasher.update(b"namix-storage-signing-v1\0");
    hasher.update(secret.as_bytes());
    hasher.finalize().into()
}
