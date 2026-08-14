//! Durable job queue: file or SQLite, with delay and a long-running worker.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use rand::RngCore;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::config::QueueSection;
use crate::queue::{JobFuture, JobResult};

const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Error)]
pub enum DurableQueueError {
    #[error("durable queue is not initialized")]
    NotInitialized,
    #[error("unsupported queue driver: {driver}")]
    UnsupportedDriver { driver: String },
    #[error("unknown queued job: {name}")]
    UnknownJob { name: String },
    #[error("queue store I/O failed")]
    Io(#[from] std::io::Error),
    #[error("queue payload encode/decode failed")]
    Codec(#[from] serde_json::Error),
    #[error("queue backend failed: {message}")]
    Backend { message: String },
}

pub type DurableQueueResult<T> = Result<T, DurableQueueError>;

impl From<DurableQueueError> for crate::AppError {
    fn from(error: DurableQueueError) -> Self {
        Self::internal(error)
    }
}

/// Persistable job: JSON payload + a registered handler.
pub trait QueuedJob: Serialize + DeserializeOwned + Send + 'static {
    const NAME: &'static str;
    fn handle(self) -> JobFuture;
}

type JobHandler = Arc<dyn Fn(serde_json::Value) -> JobFuture + Send + Sync>;

static HANDLERS: RwLock<Option<std::collections::HashMap<String, JobHandler>>> = RwLock::new(None);
static QUEUE: RwLock<Option<DurableQueue>> = RwLock::new(None);

fn handlers()
-> std::sync::RwLockWriteGuard<'static, Option<std::collections::HashMap<String, JobHandler>>> {
    HANDLERS.write().expect("queue handler lock")
}

pub fn register_job<T: QueuedJob>() {
    let mut guard = handlers();
    let map = guard.get_or_insert_with(std::collections::HashMap::new);
    map.insert(
        T::NAME.to_string(),
        Arc::new(|payload| {
            Box::pin(async move {
                let job: T = serde_json::from_value(payload).context("decode queued job")?;
                job.handle().await
            })
        }),
    );
}

fn handler_for(name: &str) -> Option<JobHandler> {
    HANDLERS.read().ok()?.as_ref()?.get(name).cloned()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub name: String,
    pub payload: serde_json::Value,
    pub available_at: u64,
    pub attempts: u32,
    pub reserved_at: Option<u64>,
    pub created_at: u64,
}

trait QueueStore: Send + Sync {
    fn push(&self, record: &JobRecord) -> DurableQueueResult<()>;
    fn claim_ready(&self, now: u64) -> DurableQueueResult<Option<JobRecord>>;
    fn delete(&self, id: &str) -> DurableQueueResult<()>;
    fn release(&self, record: &JobRecord) -> DurableQueueResult<()>;
}

#[derive(Clone)]
pub struct DurableQueue {
    driver: String,
    store: Arc<dyn QueueStore>,
}

impl DurableQueue {
    pub fn memory() -> Self {
        Self {
            driver: "memory".into(),
            store: Arc::new(MemoryStore::default()),
        }
    }

    pub fn file(path: impl AsRef<Path>) -> DurableQueueResult<Self> {
        let path = path.as_ref().to_path_buf();
        fs::create_dir_all(&path)?;
        Ok(Self {
            driver: "file".into(),
            store: Arc::new(FileStore { path }),
        })
    }

    #[cfg(feature = "sqlite")]
    pub fn sqlite(path: impl AsRef<Path>) -> DurableQueueResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let store = SqliteStore::open(&path)?;
        Ok(Self {
            driver: "sqlite".into(),
            store: Arc::new(store),
        })
    }

    pub fn driver(&self) -> &str {
        &self.driver
    }

    pub fn dispatch<T: QueuedJob>(&self, job: T) -> DurableQueueResult<String> {
        self.dispatch_later(job, Duration::ZERO)
    }

    pub fn dispatch_later<T: QueuedJob>(
        &self,
        job: T,
        delay: Duration,
    ) -> DurableQueueResult<String> {
        let now = now_secs();
        let id = new_id();
        let record = JobRecord {
            id: id.clone(),
            name: T::NAME.to_string(),
            payload: serde_json::to_value(&job)?,
            available_at: now.saturating_add(delay.as_secs()),
            attempts: 0,
            reserved_at: None,
            created_at: now,
        };
        self.store.push(&record)?;
        Ok(id)
    }

    pub async fn work_once(&self) -> Option<(String, JobResult)> {
        let record = match self.store.claim_ready(now_secs()) {
            Ok(Some(record)) => record,
            Ok(None) => return None,
            Err(error) => {
                tracing::error!(error = %error, "queue claim failed");
                return None;
            }
        };
        let name = record.name.clone();
        let Some(handler) = handler_for(&name) else {
            let _ = self.store.delete(&record.id);
            return Some((
                name.clone(),
                Err(anyhow::anyhow!(DurableQueueError::UnknownJob { name })),
            ));
        };
        let result = handler(record.payload.clone()).await;
        match &result {
            Ok(()) => {
                if let Err(error) = self.store.delete(&record.id) {
                    tracing::error!(job = %name, error = %error, "queue delete failed");
                }
            }
            Err(error) => {
                tracing::error!(job = %name, error = ?error, "queued job failed");
                let attempts = record.attempts.saturating_add(1);
                if attempts >= MAX_ATTEMPTS {
                    let _ = self.store.delete(&record.id);
                } else {
                    let retry = JobRecord {
                        attempts,
                        reserved_at: None,
                        available_at: now_secs().saturating_add(30 * u64::from(attempts)),
                        ..record
                    };
                    let _ = self.store.release(&retry);
                }
            }
        }
        Some((name, result))
    }

    pub async fn work_forever(&self) {
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("queue worker stopping");
                    break;
                }
                result = self.work_once_or_idle() => {
                    if let Some((name, Err(error))) = result {
                        tracing::error!(job = %name, error = ?error, "queued job failed");
                    }
                }
            }
        }
    }

    async fn work_once_or_idle(&self) -> Option<(String, JobResult)> {
        if let Some(result) = self.work_once().await {
            return Some(result);
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
        None
    }
}

pub fn install(queue: DurableQueue) {
    *QUEUE.write().expect("durable queue lock") = Some(queue);
}

pub fn current() -> Option<DurableQueue> {
    QUEUE.read().expect("durable queue lock").clone()
}

pub fn require() -> DurableQueueResult<DurableQueue> {
    current().ok_or(DurableQueueError::NotInitialized)
}

pub fn dispatch<T: QueuedJob>(job: T) -> DurableQueueResult<String> {
    require()?.dispatch(job)
}

pub fn dispatch_later<T: QueuedJob>(job: T, delay: Duration) -> DurableQueueResult<String> {
    require()?.dispatch_later(job, delay)
}

pub fn init(cfg: &QueueSection) -> DurableQueueResult<()> {
    let driver = cfg.driver.trim().to_ascii_lowercase();
    let queue = match driver.as_str() {
        "memory" => DurableQueue::memory(),
        "file" => DurableQueue::file(&cfg.path)?,
        "sqlite" => {
            #[cfg(feature = "sqlite")]
            {
                DurableQueue::sqlite(cfg.sqlite_path())?
            }
            #[cfg(not(feature = "sqlite"))]
            {
                return Err(DurableQueueError::UnsupportedDriver {
                    driver: "sqlite (enable namix feature sqlite)".into(),
                });
            }
        }
        other => {
            return Err(DurableQueueError::UnsupportedDriver {
                driver: other.into(),
            });
        }
    };
    crate::log::info!("queue → driver={} path={}", queue.driver(), cfg.path);
    install(queue);
    Ok(())
}

#[derive(Default)]
struct MemoryStore {
    jobs: Mutex<Vec<JobRecord>>,
}

impl QueueStore for MemoryStore {
    fn push(&self, record: &JobRecord) -> DurableQueueResult<()> {
        self.jobs
            .lock()
            .map_err(|_| DurableQueueError::Backend {
                message: "memory queue lock poisoned".into(),
            })?
            .push(record.clone());
        Ok(())
    }

    fn claim_ready(&self, now: u64) -> DurableQueueResult<Option<JobRecord>> {
        let mut jobs = self.jobs.lock().map_err(|_| DurableQueueError::Backend {
            message: "memory queue lock poisoned".into(),
        })?;
        let index = jobs
            .iter()
            .position(|job| job.available_at <= now && job.reserved_at.is_none());
        let Some(index) = index else {
            return Ok(None);
        };
        jobs[index].reserved_at = Some(now);
        Ok(Some(jobs[index].clone()))
    }

    fn delete(&self, id: &str) -> DurableQueueResult<()> {
        let mut jobs = self.jobs.lock().map_err(|_| DurableQueueError::Backend {
            message: "memory queue lock poisoned".into(),
        })?;
        jobs.retain(|job| job.id != id);
        Ok(())
    }

    fn release(&self, record: &JobRecord) -> DurableQueueResult<()> {
        let mut jobs = self.jobs.lock().map_err(|_| DurableQueueError::Backend {
            message: "memory queue lock poisoned".into(),
        })?;
        if let Some(existing) = jobs.iter_mut().find(|job| job.id == record.id) {
            *existing = record.clone();
        } else {
            jobs.push(record.clone());
        }
        Ok(())
    }
}

struct FileStore {
    path: PathBuf,
}

impl FileStore {
    fn job_path(&self, id: &str) -> PathBuf {
        self.path.join(format!("{id}.json"))
    }
}

impl QueueStore for FileStore {
    fn push(&self, record: &JobRecord) -> DurableQueueResult<()> {
        fs::create_dir_all(&self.path)?;
        atomic_write(
            &self.job_path(&record.id),
            &serde_json::to_vec_pretty(record)?,
        )
    }

    fn claim_ready(&self, now: u64) -> DurableQueueResult<Option<JobRecord>> {
        fs::create_dir_all(&self.path)?;
        let mut candidates: Vec<(u64, PathBuf, JobRecord)> = Vec::new();
        for entry in fs::read_dir(&self.path)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = fs::read(&path) else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<JobRecord>(&raw) else {
                continue;
            };
            if record.available_at <= now && record.reserved_at.is_none() {
                candidates.push((record.created_at, path, record));
            }
        }
        candidates.sort_by_key(|(created, _, _)| *created);
        for (_, path, mut record) in candidates {
            record.reserved_at = Some(now);
            let claimed = path.with_extension("run");
            if fs::rename(&path, &claimed).is_err() {
                continue;
            }
            if atomic_write(&claimed, &serde_json::to_vec_pretty(&record)?).is_err() {
                let _ = fs::rename(&claimed, &path);
                continue;
            }
            return Ok(Some(record));
        }
        Ok(None)
    }

    fn delete(&self, id: &str) -> DurableQueueResult<()> {
        let json = self.job_path(id);
        let run = json.with_extension("run");
        let _ = fs::remove_file(json);
        let _ = fs::remove_file(run);
        Ok(())
    }

    fn release(&self, record: &JobRecord) -> DurableQueueResult<()> {
        let json = self.job_path(&record.id);
        let run = json.with_extension("run");
        atomic_write(&json, &serde_json::to_vec_pretty(record)?)?;
        let _ = fs::remove_file(run);
        Ok(())
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> DurableQueueResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

#[cfg(feature = "sqlite")]
struct SqliteStore {
    path: PathBuf,
}

#[cfg(feature = "sqlite")]
impl SqliteStore {
    fn open(path: &Path) -> DurableQueueResult<Self> {
        let conn = rusqlite::Connection::open(path).map_err(sqlite_err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                payload TEXT NOT NULL,
                available_at INTEGER NOT NULL,
                attempts INTEGER NOT NULL,
                reserved_at INTEGER,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS jobs_ready ON jobs (available_at, reserved_at);",
        )
        .map_err(sqlite_err)?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn connect(&self) -> DurableQueueResult<rusqlite::Connection> {
        rusqlite::Connection::open(&self.path).map_err(sqlite_err)
    }
}

#[cfg(feature = "sqlite")]
impl QueueStore for SqliteStore {
    fn push(&self, record: &JobRecord) -> DurableQueueResult<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO jobs (id, name, payload, available_at, attempts, reserved_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                record.id,
                record.name,
                record.payload.to_string(),
                record.available_at as i64,
                record.attempts,
                record.reserved_at.map(|v| v as i64),
                record.created_at as i64,
            ],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    fn claim_ready(&self, now: u64) -> DurableQueueResult<Option<JobRecord>> {
        let mut conn = self.connect()?;
        let tx = conn.transaction().map_err(sqlite_err)?;
        let row = tx.query_row(
            "SELECT id, name, payload, available_at, attempts, reserved_at, created_at
                 FROM jobs
                 WHERE available_at <= ?1 AND reserved_at IS NULL
                 ORDER BY created_at ASC
                 LIMIT 1",
            rusqlite::params![now as i64],
            row_to_record,
        );
        let record = match row {
            Ok(record) => record,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(error) => return Err(sqlite_err(error)),
        };
        tx.execute(
            "UPDATE jobs SET reserved_at = ?1 WHERE id = ?2",
            rusqlite::params![now as i64, record.id],
        )
        .map_err(sqlite_err)?;
        tx.commit().map_err(sqlite_err)?;
        Ok(Some(record))
    }

    fn delete(&self, id: &str) -> DurableQueueResult<()> {
        self.connect()?
            .execute("DELETE FROM jobs WHERE id = ?1", rusqlite::params![id])
            .map_err(sqlite_err)?;
        Ok(())
    }

    fn release(&self, record: &JobRecord) -> DurableQueueResult<()> {
        self.connect()?
            .execute(
                "UPDATE jobs SET payload = ?1, available_at = ?2, attempts = ?3, reserved_at = NULL
                 WHERE id = ?4",
                rusqlite::params![
                    record.payload.to_string(),
                    record.available_at as i64,
                    record.attempts,
                    record.id,
                ],
            )
            .map_err(sqlite_err)?;
        Ok(())
    }
}

#[cfg(feature = "sqlite")]
fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRecord> {
    let payload: String = row.get(2)?;
    Ok(JobRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
        available_at: row.get::<_, i64>(3)? as u64,
        attempts: row.get::<_, i64>(4)? as u32,
        reserved_at: row.get::<_, Option<i64>>(5)?.map(|v| v as u64),
        created_at: row.get::<_, i64>(6)? as u64,
    })
}

#[cfg(feature = "sqlite")]
fn sqlite_err(error: rusqlite::Error) -> DurableQueueError {
    DurableQueueError::Backend {
        message: error.to_string(),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let mut random = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut random);
    format!(
        "job_{}_{}_{}",
        now_secs(),
        SEQ.fetch_add(1, Ordering::Relaxed),
        u64::from_le_bytes(random)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use std::sync::atomic::AtomicBool;

    #[derive(Serialize, Deserialize)]
    struct FlagJob {
        key: String,
    }

    static HIT: OnceLock<AtomicBool> = OnceLock::new();

    impl QueuedJob for FlagJob {
        const NAME: &'static str = "flag_job";
        fn handle(self) -> JobFuture {
            Box::pin(async move {
                HIT.get_or_init(|| AtomicBool::new(false))
                    .store(true, Ordering::SeqCst);
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn file_queue_survives_and_runs() {
        register_job::<FlagJob>();
        HIT.get_or_init(|| AtomicBool::new(false))
            .store(false, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("namix-queue-{}", new_id()));
        let queue = DurableQueue::file(&dir).unwrap();
        queue.dispatch(FlagJob { key: "x".into() }).unwrap();
        let (name, result) = queue.work_once().await.expect("job");
        assert_eq!(name, "flag_job");
        result.unwrap();
        assert!(HIT.get().unwrap().load(Ordering::SeqCst));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn delayed_job_is_not_ready() {
        register_job::<FlagJob>();
        let queue = DurableQueue::memory();
        queue
            .dispatch_later(
                FlagJob {
                    key: "later".into(),
                },
                Duration::from_secs(60 * 60),
            )
            .unwrap();
        assert!(queue.work_once().await.is_none());
    }
}
