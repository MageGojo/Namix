//! 邮件门面（Laravel `Mail` 子集）。
//!
//! ```ignore
//! use namix::mail::{Mail, MailMessage};
//!
//! Mail::send(MailMessage::new("a@b.com", "Hi").text("body"))?;
//! Mail::receive(MailMessage::new("user@x.com", "Inbound").text("…")); // 入站落库
//! let out = Mail::outbox();
//! let inbox = Mail::inbox();
//! ```

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::MailSection;

static MAIL: OnceLock<MailRuntime> = OnceLock::new();

#[derive(Debug, Error)]
pub enum MailError {
    #[error("mail is not initialized (Boot did not call mail::init)")]
    NotInitialized,
    #[error("unsupported mail driver: {driver}")]
    UnsupportedDriver { driver: String },
    #[error("inbound mail missing from")]
    MissingSender,
    #[error("mail store lock poisoned")]
    StoreLockPoisoned,
    #[error("mail store I/O failed")]
    Io(#[source] std::io::Error),
    #[error("mail serialization failed")]
    Serialize(#[source] serde_json::Error),
}

pub type MailResult<T> = Result<T, MailError>;

impl From<MailError> for crate::AppError {
    fn from(error: MailError) -> Self {
        match error {
            MailError::MissingSender => Self::validation("from", "inbound mail missing from"),
            MailError::UnsupportedDriver { .. } => Self::bad_request("unsupported mail driver"),
            other => Self::internal(other),
        }
    }
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
    /// `out` = 发出；`in` = 收取。
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
    lock: Mutex<()>,
}

/// 静态门面。
pub struct Mail;

impl Mail {
    pub fn send(mut msg: MailMessage) -> MailResult<()> {
        let rt = runtime()?;
        if msg.from.is_empty() {
            msg.from = rt.from.clone();
        }
        msg.direction = "out".into();
        msg.at = now_secs();
        if msg.id.is_empty() {
            msg.id = new_id("mail");
        }

        match rt.driver.as_str() {
            "log" | "file" => {
                crate::log::info!(
                    "mail:send driver={} to={} subject={}",
                    rt.driver,
                    msg.to,
                    msg.subject
                );
                append_jsonl(&rt.store.join("outbox.jsonl"), &msg, &rt.lock)?;
                Ok(())
            }
            other => Err(MailError::UnsupportedDriver {
                driver: other.into(),
            }),
        }
    }

    /// 入站邮件（webhook / IMAP 拉取后写入）。
    pub fn receive(mut msg: MailMessage) -> MailResult<()> {
        let rt = runtime()?;
        if msg.from.is_empty() {
            return Err(MailError::MissingSender);
        }
        msg.direction = "in".into();
        msg.at = now_secs();
        if msg.id.is_empty() {
            msg.id = new_id("mail");
        }
        crate::log::info!(
            "mail:receive from={} to={} subject={}",
            msg.from,
            msg.to,
            msg.subject
        );
        append_jsonl(&rt.store.join("inbox.jsonl"), &msg, &rt.lock)?;
        Ok(())
    }

    pub fn outbox() -> Vec<MailMessage> {
        read_jsonl(
            runtime()
                .ok()
                .map(|r| r.store.join("outbox.jsonl"))
                .as_deref(),
        )
    }

    pub fn inbox() -> Vec<MailMessage> {
        read_jsonl(
            runtime()
                .ok()
                .map(|r| r.store.join("inbox.jsonl"))
                .as_deref(),
        )
    }

    pub fn from_address() -> String {
        runtime().map(|r| r.from.clone()).unwrap_or_default()
    }

    pub fn driver() -> String {
        runtime()
            .map(|r| r.driver.clone())
            .unwrap_or_else(|_| "uninitialized".into())
    }
}

pub fn init(cfg: &MailSection) {
    let store = PathBuf::from(if cfg.store.trim().is_empty() {
        "./storage/mail"
    } else {
        cfg.store.trim()
    });
    let _ = fs::create_dir_all(&store);
    let _ = MAIL.set(MailRuntime {
        driver: cfg.driver.trim().to_ascii_lowercase(),
        from: if cfg.from.trim().is_empty() {
            "noreply@namix.local".into()
        } else {
            cfg.from.trim().into()
        },
        store,
        lock: Mutex::new(()),
    });
    crate::log::info!(
        "mail → driver={} from={} store={}",
        Mail::driver(),
        Mail::from_address(),
        runtime()
            .map(|r| r.store.display().to_string())
            .unwrap_or_default()
    );
}

fn runtime() -> MailResult<&'static MailRuntime> {
    MAIL.get().ok_or(MailError::NotInitialized)
}

fn append_jsonl(path: &Path, msg: &MailMessage, lock: &Mutex<()>) -> MailResult<()> {
    let _guard = lock.lock().map_err(|_| MailError::StoreLockPoisoned)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(MailError::Io)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(MailError::Io)?;
    let line = serde_json::to_string(msg).map_err(MailError::Serialize)?;
    writeln!(file, "{line}").map_err(MailError::Io)
}

fn read_jsonl(path: Option<&Path>) -> Vec<MailMessage> {
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<MailMessage>(l).ok())
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
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
