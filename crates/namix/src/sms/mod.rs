//! 短信门面（Laravel Notification / SMS 子集）。
//!
//! ```ignore
//! use namix::sms::{Sms, SmsMessage};
//! Sms::send(SmsMessage::new("13800138000", "您的验证码是 123456"))?;
//! ```

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::SmsSection;

static SMS: OnceLock<SmsRuntime> = OnceLock::new();
static TRANSPORTS: OnceLock<RwLock<HashMap<String, Arc<dyn SmsTransport>>>> = OnceLock::new();

pub type SmsTransportError = Box<dyn StdError + Send + Sync + 'static>;
pub type SmsTransportResult<T> = Result<T, SmsTransportError>;

/// Adapter contract for Aliyun / Twilio / etc. Register before `Boot::run`.
pub trait SmsTransport: Send + Sync + 'static {
    fn send(&self, message: &SmsMessage) -> SmsTransportResult<()>;
}

pub fn register_transport(driver: impl AsRef<str>, transport: impl SmsTransport) -> SmsResult<()> {
    let driver = driver.as_ref().trim().to_ascii_lowercase();
    if driver.is_empty() || matches!(driver.as_str(), "log" | "file") {
        return Err(SmsError::UnsupportedDriver { driver });
    }
    let mut transports = transports()
        .write()
        .map_err(|_| SmsError::StoreLockPoisoned)?;
    if transports.contains_key(&driver) {
        return Err(SmsError::UnsupportedDriver { driver });
    }
    transports.insert(driver, Arc::new(transport));
    Ok(())
}

fn transports() -> &'static RwLock<HashMap<String, Arc<dyn SmsTransport>>> {
    TRANSPORTS.get_or_init(|| RwLock::new(HashMap::new()))
}

#[derive(Debug, Error)]
pub enum SmsError {
    #[error("sms is not initialized (Boot did not call sms::init)")]
    NotInitialized,
    #[error("unsupported sms driver: {driver}")]
    UnsupportedDriver { driver: String },
    #[error("invalid phone number")]
    InvalidPhone,
    #[error("sms store lock poisoned")]
    StoreLockPoisoned,
    #[error("sms OTP lock poisoned")]
    OtpLockPoisoned,
    #[error("sms store I/O failed")]
    Io(#[source] std::io::Error),
    #[error("sms serialization failed")]
    Serialize(#[source] serde_json::Error),
}

pub type SmsResult<T> = Result<T, SmsError>;

impl From<SmsError> for crate::AppError {
    fn from(error: SmsError) -> Self {
        match error {
            SmsError::InvalidPhone => Self::validation("phone", "phone.invalid"),
            SmsError::UnsupportedDriver { .. } => Self::bad_request("unsupported sms driver"),
            other => Self::internal(other),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsMessage {
    pub id: String,
    pub to: String,
    pub body: String,
    #[serde(default)]
    pub from: String,
    pub at: u64,
}

impl SmsMessage {
    pub fn new(to: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: format!("sms_{}", now_secs()),
            to: to.into(),
            body: body.into(),
            from: String::new(),
            at: now_secs(),
        }
    }
}

#[derive(Debug, Clone)]
struct OtpEntry {
    code: String,
    expires_at: u64,
}

struct SmsRuntime {
    driver: String,
    from: String,
    store: PathBuf,
    lock: Mutex<()>,
    otps: Mutex<HashMap<String, OtpEntry>>,
    /// 开发用：是否在日志里打印验证码明文。
    log_otp: bool,
}

pub struct Sms;

impl Sms {
    pub fn send(mut msg: SmsMessage) -> SmsResult<()> {
        let rt = runtime()?;
        if msg.from.is_empty() {
            msg.from = rt.from.clone();
        }
        msg.at = now_secs();
        match rt.driver.as_str() {
            "log" | "file" => {
                crate::log::info!(
                    "sms:send driver={} to={} bytes={}",
                    rt.driver,
                    msg.to,
                    msg.body.len()
                );
                append_jsonl(&rt.store.join("sent.jsonl"), &msg, &rt.lock)?;
                Ok(())
            }
            other => {
                let transport = transports()
                    .read()
                    .map_err(|_| SmsError::StoreLockPoisoned)?
                    .get(other)
                    .cloned();
                let Some(transport) = transport else {
                    return Err(SmsError::UnsupportedDriver {
                        driver: other.into(),
                    });
                };
                transport.send(&msg).map_err(|source| {
                    crate::log::error!("sms transport `{other}` failed: {source}");
                    SmsError::UnsupportedDriver {
                        driver: other.into(),
                    }
                })?;
                append_jsonl(&rt.store.join("sent.jsonl"), &msg, &rt.lock)?;
                Ok(())
            }
        }
    }

    /// 发送 6 位验证码（默认 5 分钟有效）。
    pub fn send_code(phone: &str) -> SmsResult<String> {
        let phone = normalize_phone(phone)?;
        let code = generate_code();
        let rt = runtime()?;
        {
            let mut otps = rt.otps.lock().map_err(|_| SmsError::OtpLockPoisoned)?;
            otps.insert(
                phone.clone(),
                OtpEntry {
                    code: code.clone(),
                    expires_at: now_secs() + 300,
                },
            );
        }
        let body = format!("【Namix】您的验证码是 {code}，5 分钟内有效。");
        Self::send(SmsMessage::new(&phone, body))?;
        if rt.log_otp {
            crate::log::info!("sms:otp phone={phone} code={code}");
        }
        Ok(code)
    }

    pub fn verify_code(phone: &str, code: &str) -> SmsResult<bool> {
        let phone = normalize_phone(phone)?;
        let code = code.trim();
        if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
            return Ok(false);
        }
        let rt = runtime()?;
        let mut otps = rt.otps.lock().map_err(|_| SmsError::OtpLockPoisoned)?;
        let Some(entry) = otps.get(&phone) else {
            return Ok(false);
        };
        if entry.expires_at < now_secs() {
            otps.remove(&phone);
            return Ok(false);
        }
        let ok = constant_time_eq(entry.code.as_bytes(), code.as_bytes());
        if ok {
            otps.remove(&phone);
        }
        Ok(ok)
    }

    pub fn sent() -> Vec<SmsMessage> {
        read_jsonl(
            runtime()
                .ok()
                .map(|r| r.store.join("sent.jsonl"))
                .as_deref(),
        )
    }

    pub fn driver() -> String {
        runtime()
            .map(|r| r.driver.clone())
            .unwrap_or_else(|_| "uninitialized".into())
    }
}

pub fn init(cfg: &SmsSection) {
    let store = PathBuf::from(if cfg.store.trim().is_empty() {
        "./storage/sms"
    } else {
        cfg.store.trim()
    });
    let _ = fs::create_dir_all(&store);
    let _ = SMS.set(SmsRuntime {
        driver: cfg.driver.trim().to_ascii_lowercase(),
        from: if cfg.from.trim().is_empty() {
            "Namix".into()
        } else {
            cfg.from.trim().into()
        },
        store,
        lock: Mutex::new(()),
        otps: Mutex::new(HashMap::new()),
        log_otp: cfg.log_otp,
    });
    crate::log::info!(
        "sms → driver={} from={} log_otp={}",
        Sms::driver(),
        runtime().map(|r| r.from.clone()).unwrap_or_default(),
        cfg.log_otp
    );
}

fn runtime() -> SmsResult<&'static SmsRuntime> {
    SMS.get().ok_or(SmsError::NotInitialized)
}

fn normalize_phone(phone: &str) -> SmsResult<String> {
    let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 8 || digits.len() > 15 {
        return Err(SmsError::InvalidPhone);
    }
    Ok(digits)
}

fn generate_code() -> String {
    let mut bytes = [0u8; 4];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let len = left.len().max(right.len());
    for i in 0..len {
        diff |= usize::from(*left.get(i).unwrap_or(&0) ^ *right.get(i).unwrap_or(&0));
    }
    diff == 0
}

fn append_jsonl(path: &Path, msg: &SmsMessage, lock: &Mutex<()>) -> SmsResult<()> {
    let _guard = lock.lock().map_err(|_| SmsError::StoreLockPoisoned)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(SmsError::Io)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(SmsError::Io)?;
    let line = serde_json::to_string(msg).map_err(SmsError::Serialize)?;
    writeln!(file, "{line}").map_err(SmsError::Io)
}

fn read_jsonl(path: Option<&Path>) -> Vec<SmsMessage> {
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<SmsMessage>(l).ok())
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
