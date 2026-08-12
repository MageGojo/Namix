//! Mail facade with inspectable development drivers and pluggable production
//! transports.
//!
//! A production adapter registers itself before `Boot::run` and selects the
//! same name in `[mail].driver`:
//!
//! ```ignore
//! namix::mail::register_transport("smtp", SmtpTransport::new(...))?;
//! // namix.toml: [mail] driver = "smtp"
//! ```

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::MailSection;
use crate::queue::{Job, JobFuture, Queue, QueueResult};

static MAIL: OnceLock<MailRuntime> = OnceLock::new();
static TRANSPORTS: OnceLock<RwLock<HashMap<String, Arc<dyn MailTransport>>>> = OnceLock::new();

pub type MailTransportError = Box<dyn StdError + Send + Sync + 'static>;
pub type MailTransportResult<T> = Result<T, MailTransportError>;

#[derive(Debug, Error)]
pub enum MailError {
    #[error("mail is not initialized (Boot did not call mail::init)")]
    NotInitialized,
    #[error("unsupported mail driver: {driver}")]
    UnsupportedDriver { driver: String },
    #[error("invalid mail transport name: {driver}")]
    InvalidTransportName { driver: String },
    #[error("mail transport already registered: {driver}")]
    TransportAlreadyRegistered { driver: String },
    #[error("mail transport registry lock poisoned")]
    TransportRegistryLockPoisoned,
    #[error("mail runtime is already initialized with a different configuration")]
    AlreadyInitialized,
    #[error("mail transport `{driver}` failed")]
    Transport {
        driver: String,
        #[source]
        source: MailTransportError,
    },
    #[error("inbound mail missing from")]
    MissingSender,
    #[error("mail store lock poisoned")]
    StoreLockPoisoned,
    #[error("mail store {operation} failed for {path}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("mail serialization failed")]
    Serialize(#[source] serde_json::Error),
    #[error("mail journal entry {line} in {path} is invalid")]
    Deserialize {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
}

pub type MailResult<T> = Result<T, MailError>;

impl From<MailError> for crate::AppError {
    fn from(error: MailError) -> Self {
        match error {
            MailError::MissingSender => Self::validation("from", "inbound mail missing from"),
            MailError::UnsupportedDriver { .. } | MailError::InvalidTransportName { .. } => {
                Self::bad_request(error.to_string())
            }
            other => Self::internal(other),
        }
    }
}

/// Adapter contract for SMTP, transactional-email APIs, and local relays.
/// Provider errors remain attached as concrete source errors.
pub trait MailTransport: Send + Sync + 'static {
    fn send(&self, message: &MailMessage) -> MailTransportResult<()>;
}

/// Register a named custom transport before [`init`] / `Boot::run`.
pub fn register_transport(
    driver: impl AsRef<str>,
    transport: impl MailTransport,
) -> MailResult<()> {
    let driver = normalize_driver(driver.as_ref())?;
    if matches!(driver.as_str(), "log" | "file") {
        return Err(MailError::TransportAlreadyRegistered { driver });
    }

    let mut transports = transports()
        .write()
        .map_err(|_| MailError::TransportRegistryLockPoisoned)?;
    if transports.contains_key(&driver) {
        return Err(MailError::TransportAlreadyRegistered { driver });
    }
    transports.insert(driver, Arc::new(transport));
    Ok(())
}

fn transports() -> &'static RwLock<HashMap<String, Arc<dyn MailTransport>>> {
    TRANSPORTS.get_or_init(|| RwLock::new(HashMap::new()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailMessage {
    pub id: String,
    pub from: String,
    pub to: String,
    pub subject: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub html: String,
    pub at: u64,
    /// `out` = sent; `in` = received.
    pub direction: String,
}

impl MailMessage {
    pub fn new(to: impl Into<String>, subject: impl Into<String>) -> Self {
        Self {
            id: new_id("mail"),
            from: String::new(),
            to: to.into(),
            subject: subject.into(),
            text: String::new(),
            html: String::new(),
            at: now_secs(),
            direction: "out".into(),
        }
    }

    pub fn from_addr(mut self, from: impl Into<String>) -> Self {
        self.from = from.into();
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    pub fn html(mut self, html: impl Into<String>) -> Self {
        self.html = html.into();
        self
    }
}

struct MailRuntime {
    driver: String,
    from: String,
    store: PathBuf,
    transport: Option<Arc<dyn MailTransport>>,
    lock: Mutex<()>,
}

/// Static facade used by controllers, Actions, and queue jobs.
pub struct Mail;

impl Mail {
    pub fn send(mut message: MailMessage) -> MailResult<()> {
        let runtime = runtime()?;
        if message.from.is_empty() {
            message.from = runtime.from.clone();
        }
        message.direction = "out".into();
        message.at = now_secs();
        if message.id.is_empty() {
            message.id = new_id("mail");
        }

        if let Some(transport) = &runtime.transport {
            transport
                .send(&message)
                .map_err(|source| MailError::Transport {
                    driver: runtime.driver.clone(),
                    source,
                })?;
        }

        crate::log::info!(
            "mail:send driver={} to={} subject={}",
            runtime.driver,
            message.to,
            message.subject
        );
        append_jsonl(&runtime.store.join("outbox.jsonl"), &message, &runtime.lock)
    }

    /// Record inbound mail after an application's webhook/IMAP adapter has
    /// verified and normalized it.
    pub fn receive(mut message: MailMessage) -> MailResult<()> {
        let runtime = runtime()?;
        if message.from.is_empty() {
            return Err(MailError::MissingSender);
        }
        message.direction = "in".into();
        message.at = now_secs();
        if message.id.is_empty() {
            message.id = new_id("mail");
        }
        crate::log::info!(
            "mail:receive from={} to={} subject={}",
            message.from,
            message.to,
            message.subject
        );
        append_jsonl(&runtime.store.join("inbox.jsonl"), &message, &runtime.lock)
    }

    pub fn try_outbox() -> MailResult<Vec<MailMessage>> {
        let runtime = runtime()?;
        read_jsonl(&runtime.store.join("outbox.jsonl"))
    }

    pub fn outbox() -> Vec<MailMessage> {
        match Self::try_outbox() {
            Ok(messages) => messages,
            Err(error) => {
                tracing::error!(error = ?error, "mail outbox read failed");
                Vec::new()
            }
        }
    }

    pub fn try_inbox() -> MailResult<Vec<MailMessage>> {
        let runtime = runtime()?;
        read_jsonl(&runtime.store.join("inbox.jsonl"))
    }

    pub fn inbox() -> Vec<MailMessage> {
        match Self::try_inbox() {
            Ok(messages) => messages,
            Err(error) => {
                tracing::error!(error = ?error, "mail inbox read failed");
                Vec::new()
            }
        }
    }

    pub fn try_from_address() -> MailResult<String> {
        Ok(runtime()?.from.clone())
    }

    pub fn from_address() -> String {
        match Self::try_from_address() {
            Ok(address) => address,
            Err(error) => {
                tracing::error!(error = ?error, "mail sender lookup failed");
                String::new()
            }
        }
    }

    pub fn try_driver() -> MailResult<String> {
        Ok(runtime()?.driver.clone())
    }

    pub fn driver() -> String {
        match Self::try_driver() {
            Ok(driver) => driver,
            Err(error) => {
                tracing::error!(error = ?error, "mail driver lookup failed");
                "uninitialized".into()
            }
        }
    }

    pub fn job(message: MailMessage) -> MailJob {
        MailJob::new(message)
    }

    pub async fn dispatch(queue: &Queue, message: MailMessage) -> QueueResult<()> {
        queue.dispatch(Self::job(message)).await
    }
}

/// Boot-compatible initializer. For callers that need to handle startup
/// errors directly, use [`try_init`].
pub fn init(config: &MailSection) {
    if let Err(error) = try_init(config) {
        panic!("mail initialization failed: {error}");
    }
}

pub fn try_init(config: &MailSection) -> MailResult<()> {
    let driver = normalize_driver(&config.driver)?;
    let store = PathBuf::from(if config.store.trim().is_empty() {
        "./storage/mail"
    } else {
        config.store.trim()
    });
    fs::create_dir_all(&store).map_err(|source| MailError::Io {
        operation: "create directory",
        path: store.clone(),
        source,
    })?;

    let transport = if matches!(driver.as_str(), "log" | "file") {
        None
    } else {
        Some(
            transports()
                .read()
                .map_err(|_| MailError::TransportRegistryLockPoisoned)?
                .get(&driver)
                .cloned()
                .ok_or_else(|| MailError::UnsupportedDriver {
                    driver: driver.clone(),
                })?,
        )
    };

    let from = if config.from.trim().is_empty() {
        "noreply@namix.local".into()
    } else {
        config.from.trim().into()
    };

    if let Some(runtime) = MAIL.get() {
        return if runtime.driver == driver && runtime.from == from && runtime.store == store {
            Ok(())
        } else {
            Err(MailError::AlreadyInitialized)
        };
    }

    MAIL.set(MailRuntime {
        driver,
        from,
        store,
        transport,
        lock: Mutex::new(()),
    })
    .map_err(|_| MailError::AlreadyInitialized)?;

    let runtime = runtime()?;
    crate::log::info!(
        "mail → driver={} from={} store={}",
        runtime.driver,
        runtime.from,
        runtime.store.display()
    );
    Ok(())
}

fn normalize_driver(driver: &str) -> MailResult<String> {
    let driver = driver.trim().to_ascii_lowercase();
    if driver.is_empty()
        || !driver
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(MailError::InvalidTransportName { driver });
    }
    Ok(driver)
}

fn runtime() -> MailResult<&'static MailRuntime> {
    MAIL.get().ok_or(MailError::NotInitialized)
}

fn append_jsonl(path: &Path, message: &MailMessage, lock: &Mutex<()>) -> MailResult<()> {
    let _guard = lock.lock().map_err(|_| MailError::StoreLockPoisoned)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| MailError::Io {
            operation: "create directory",
            path: parent.to_owned(),
            source,
        })?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| MailError::Io {
            operation: "open",
            path: path.to_owned(),
            source,
        })?;
    let line = serde_json::to_string(message).map_err(MailError::Serialize)?;
    writeln!(file, "{line}").map_err(|source| MailError::Io {
        operation: "append",
        path: path.to_owned(),
        source,
    })
}

fn read_jsonl(path: &Path) -> MailResult<Vec<MailMessage>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(MailError::Io {
                operation: "read",
                path: path.to_owned(),
                source,
            });
        }
    };

    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<MailMessage>(line).map_err(|source| MailError::Deserialize {
                path: path.to_owned(),
                line: index + 1,
                source,
            })
        })
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn new_id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    format!(
        "{prefix}_{}_{}",
        now_secs(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

pub struct MailJob {
    message: MailMessage,
}

impl MailJob {
    pub fn new(message: MailMessage) -> Self {
        Self { message }
    }
}

impl Job for MailJob {
    fn name(&self) -> &'static str {
        "mail.send"
    }

    fn handle(self: Box<Self>) -> JobFuture {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || Mail::send(self.message))
                .await
                .context("mail worker join")??;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingTransport(Arc<AtomicUsize>);

    impl MailTransport for CountingTransport {
        fn send(&self, message: &MailMessage) -> MailTransportResult<()> {
            if message.to.is_empty() {
                return Err(Box::new(io::Error::other("recipient missing")));
            }
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn custom_transport_supports_direct_and_queued_delivery() {
        let suffix = format!("{}-{}", std::process::id(), now_secs());
        let driver = format!("test-provider-{suffix}");
        let store = std::env::temp_dir().join(format!("namix-mail-{suffix}"));
        let delivered = Arc::new(AtomicUsize::new(0));
        register_transport(&driver, CountingTransport(Arc::clone(&delivered))).unwrap();
        try_init(&MailSection {
            driver,
            from: "framework@example.test".into(),
            store: store.display().to_string(),
        })
        .unwrap();

        Mail::send(MailMessage::new("direct@example.test", "Direct")).unwrap();
        let queue = Queue::memory(1);
        Mail::dispatch(&queue, MailMessage::new("queued@example.test", "Queued"))
            .await
            .unwrap();
        let (name, result) = queue.work_once().await.unwrap();
        assert_eq!(name, "mail.send");
        result.unwrap();

        assert_eq!(delivered.load(Ordering::SeqCst), 2);
        let messages = Mail::try_outbox().unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|message| {
            message.from == "framework@example.test" && message.direction == "out"
        }));

        let transport_error = Mail::send(MailMessage::new("", "Missing recipient")).unwrap_err();
        assert!(matches!(transport_error, MailError::Transport { .. }));
        assert!(format!("{transport_error:?}").contains("recipient missing"));
        assert_eq!(Mail::try_outbox().unwrap().len(), 2);

        let journal = store.join("outbox.jsonl");
        OpenOptions::new()
            .append(true)
            .open(&journal)
            .unwrap()
            .write_all(b"not-json\n")
            .unwrap();
        assert!(matches!(
            Mail::try_outbox().unwrap_err(),
            MailError::Deserialize { line: 3, .. }
        ));
        let _ = fs::remove_dir_all(store);
    }

    #[test]
    fn transport_names_are_validated() {
        let error = register_transport("../smtp", CountingTransport(Arc::new(AtomicUsize::new(0))))
            .unwrap_err();
        assert!(matches!(error, MailError::InvalidTransportName { .. }));
    }
}
