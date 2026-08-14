//! 多应用配置：一份 namix.toml。
//!
//! ```toml
//! [apps.www]
//! port = 3000
//! https = true          # 本地自签 HTTPS
//! https_port = 3443
//! lan = false
//! hosts = ["www.yyy.com"]
//! ```

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

use crate::Server;

#[derive(Debug, Clone, Deserialize)]
pub struct NamixToml {
    #[serde(default)]
    pub apps: BTreeMap<String, AppConfig>,
    #[serde(default)]
    pub features: FeaturesSection,
    #[serde(default)]
    pub database: DatabaseSection,
    #[serde(default)]
    pub mail: MailSection,
    #[serde(default)]
    pub sms: SmsSection,
    #[serde(default)]
    pub queue: QueueSection,
    #[serde(default)]
    pub i18n: I18nSection,
    #[serde(default)]
    pub storage: StorageSection,
    #[serde(default)]
    pub security: SecuritySection,
    #[serde(default)]
    pub session: SessionSection,
}

/// `[session]` — authenticated session persistence and token lifetimes.
///
/// ```toml
/// [session]
/// driver = "memory"           # memory | file | redis
/// path = "./storage/sessions" # file driver root (shared via dist/data)
/// lifetime_secs = 604800      # cookie / opaque session (7d)
/// jwt_lifetime_secs = 3600    # Bearer JWT access token (1h)
/// ```
///
/// `memory` is the development default and is **not** safe across overlapping
/// processes. Production rolling updates require `file` (shared data plane) or
/// `redis` (application-wired [`crate::RedisSessionStore`]).
#[derive(Debug, Clone, Deserialize)]
pub struct SessionSection {
    #[serde(default = "default_session_driver")]
    pub driver: String,
    #[serde(default = "default_session_path")]
    pub path: String,
    /// Absolute lifetime for cookie / opaque session tokens (seconds).
    #[serde(default = "default_session_lifetime_secs")]
    pub lifetime_secs: u64,
    /// Lifetime for HS256 JWT access tokens issued for API Bearer auth.
    #[serde(default = "default_jwt_lifetime_secs")]
    pub jwt_lifetime_secs: u64,
}

impl Default for SessionSection {
    fn default() -> Self {
        Self {
            driver: default_session_driver(),
            path: default_session_path(),
            lifetime_secs: default_session_lifetime_secs(),
            jwt_lifetime_secs: default_jwt_lifetime_secs(),
        }
    }
}

fn default_session_driver() -> String {
    "memory".into()
}

fn default_session_path() -> String {
    "./storage/sessions".into()
}

fn default_session_lifetime_secs() -> u64 {
    60 * 60 * 24 * 7
}

fn default_jwt_lifetime_secs() -> u64 {
    60 * 60
}

/// `[security]` — browser protection, runtime environment, and secrets.
///
/// `session_secret` may also be supplied through `NAMIX_SESSION_SECRET`.  It
/// is mandatory in production and feeds application-level signed sessions.
#[derive(Debug, Clone, Deserialize)]
pub struct SecuritySection {
    #[serde(default = "default_environment")]
    pub environment: String,
    #[serde(default = "default_true")]
    pub csrf: bool,
    #[serde(default)]
    pub csrf_trusted_origins: Vec<String>,
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    /// Set when a trusted edge proxy terminates public HTTPS and forwards to
    /// Namix over loopback/private networking.
    #[serde(default)]
    pub tls_terminated_by_proxy: bool,
    #[serde(default)]
    pub session_secret: Option<String>,
    #[serde(default)]
    pub rate_limit: RateLimitSection,
}

impl Default for SecuritySection {
    fn default() -> Self {
        Self {
            environment: default_environment(),
            csrf: true,
            csrf_trusted_origins: Vec::new(),
            trusted_proxies: Vec::new(),
            tls_terminated_by_proxy: false,
            session_secret: None,
            rate_limit: RateLimitSection::default(),
        }
    }
}

fn default_environment() -> String {
    "development".into()
}

/// Default budgets intentionally protect expensive browser paths without
/// constraining read-only traffic.  A value of `0` disables that budget.
#[derive(Debug, Clone, Deserialize)]
pub struct RateLimitSection {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_rate_window")]
    pub window_seconds: u64,
    #[serde(default = "default_login_limit")]
    pub login: usize,
    #[serde(default = "default_registration_limit")]
    pub registration: usize,
    #[serde(default = "default_action_limit")]
    pub action: usize,
    #[serde(default = "default_upload_limit")]
    pub upload: usize,
}

impl Default for RateLimitSection {
    fn default() -> Self {
        Self {
            enabled: true,
            window_seconds: default_rate_window(),
            login: default_login_limit(),
            registration: default_registration_limit(),
            action: default_action_limit(),
            upload: default_upload_limit(),
        }
    }
}

fn default_rate_window() -> u64 {
    60
}
fn default_login_limit() -> usize {
    5
}
fn default_registration_limit() -> usize {
    3
}
fn default_action_limit() -> usize {
    60
}
fn default_upload_limit() -> usize {
    10
}

/// Startup configuration failures collected before any listener is opened.
///
/// Parsing retains TOML's line/column diagnostic as a source; validation
/// retains all independent configuration mistakes so an operator fixes them
/// in one edit instead of one restart at a time.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("namix.toml parse failed")]
    Parse {
        #[source]
        source: toml::de::Error,
    },
    #[error("{}", messages.join("; "))]
    Validation { messages: Vec<String> },
}

impl ConfigError {
    fn one(message: impl Into<String>) -> Self {
        Self::Validation {
            messages: vec![message.into()],
        }
    }

    pub fn messages(&self) -> &[String] {
        match self {
            Self::Parse { .. } => &[],
            Self::Validation { messages } => messages,
        }
    }
}
static SESSION_SECRET: OnceLock<String> = OnceLock::new();
static SESSION_COOKIE_SECURE: OnceLock<bool> = OnceLock::new();
static SESSION_LIFETIME: OnceLock<Duration> = OnceLock::new();
static JWT_LIFETIME: OnceLock<Duration> = OnceLock::new();

/// Runtime session-signing key installed by [`crate::Boot`].
pub fn session_secret() -> Option<&'static str> {
    SESSION_SECRET.get().map(String::as_str)
}

/// Whether the framework installed secure session cookies for this runtime.
pub fn session_cookie_secure() -> bool {
    *SESSION_COOKIE_SECURE.get().unwrap_or(&false)
}

/// Cookie / opaque session absolute lifetime (from `[session].lifetime_secs`).
pub fn session_lifetime() -> Duration {
    SESSION_LIFETIME
        .get()
        .copied()
        .unwrap_or_else(|| Duration::from_secs(default_session_lifetime_secs()))
}

/// JWT access-token lifetime (from `[session].jwt_lifetime_secs`).
pub fn jwt_lifetime() -> Duration {
    JWT_LIFETIME
        .get()
        .copied()
        .unwrap_or_else(|| Duration::from_secs(default_jwt_lifetime_secs()))
}

pub(crate) fn install_session_secret(value: Option<String>, secure_cookie: bool) {
    let value = value
        .or_else(|| std::env::var("NAMIX_SESSION_SECRET").ok())
        .filter(|secret| !secret.trim().is_empty());
    if let Some(value) = value {
        let _ = SESSION_SECRET.set(value);
    }
    let _ = SESSION_COOKIE_SECURE.set(secure_cookie);
}

pub(crate) fn install_session_lifetimes(session: &SessionSection) {
    let _ = SESSION_LIFETIME.set(Duration::from_secs(session.lifetime_secs.max(1)));
    let _ = JWT_LIFETIME.set(Duration::from_secs(session.jwt_lifetime_secs.max(1)));
}

/// `[mail]` — 邮件发送 / 入站落库。
///
/// ```toml
/// [mail]
/// driver = "log"   # log | file（目前均写 storage + 打日志；后续可加 smtp）
/// from = "noreply@namix.local"
/// store = "./storage/mail"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct MailSection {
    #[serde(default = "default_mail_driver")]
    pub driver: String,
    #[serde(default = "default_mail_from")]
    pub from: String,
    #[serde(default = "default_mail_store")]
    pub store: String,
}

impl Default for MailSection {
    fn default() -> Self {
        Self {
            driver: default_mail_driver(),
            from: default_mail_from(),
            store: default_mail_store(),
        }
    }
}

fn default_mail_driver() -> String {
    "log".into()
}
fn default_mail_from() -> String {
    "noreply@namix.local".into()
}
fn default_mail_store() -> String {
    "./storage/mail".into()
}

/// `[sms]` — 短信发送 / OTP。
///
/// ```toml
/// [sms]
/// driver = "log"
/// from = "Namix"
/// store = "./storage/sms"
/// log_otp = true   # 开发：日志打印验证码
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct SmsSection {
    #[serde(default = "default_sms_driver")]
    pub driver: String,
    #[serde(default = "default_sms_from")]
    pub from: String,
    #[serde(default = "default_sms_store")]
    pub store: String,
    #[serde(default)]
    pub log_otp: bool,
}

impl Default for SmsSection {
    fn default() -> Self {
        Self {
            driver: default_sms_driver(),
            from: default_sms_from(),
            store: default_sms_store(),
            log_otp: false,
        }
    }
}

fn default_sms_driver() -> String {
    "log".into()
}
fn default_sms_from() -> String {
    "Namix".into()
}
fn default_sms_store() -> String {
    "./storage/sms".into()
}

/// `[queue]` — durable jobs for `nx work` (no Redis).
///
/// ```toml
/// [queue]
/// driver = "file"            # memory | file | sqlite
/// path = "./storage/queue"   # file dir, or sqlite file / dir
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct QueueSection {
    #[serde(default = "default_queue_driver")]
    pub driver: String,
    #[serde(default = "default_queue_path")]
    pub path: String,
}

impl Default for QueueSection {
    fn default() -> Self {
        Self {
            driver: default_queue_driver(),
            path: default_queue_path(),
        }
    }
}

impl QueueSection {
    pub fn sqlite_path(&self) -> std::path::PathBuf {
        let path = std::path::PathBuf::from(&self.path);
        if path.extension().is_some() {
            path
        } else {
            path.with_extension("sqlite")
        }
    }
}

fn default_queue_driver() -> String {
    "file".into()
}
fn default_queue_path() -> String {
    "./storage/queue".into()
}

/// `[i18n]` — JSON dictionaries under `lang/`.
#[derive(Debug, Clone, Deserialize)]
pub struct I18nSection {
    #[serde(default = "default_i18n_locale")]
    pub locale: String,
    #[serde(default = "default_i18n_path")]
    pub path: String,
}

impl Default for I18nSection {
    fn default() -> Self {
        Self {
            locale: default_i18n_locale(),
            path: default_i18n_path(),
        }
    }
}

fn default_i18n_locale() -> String {
    "zh-CN".into()
}
fn default_i18n_path() -> String {
    "./lang".into()
}

/// `[storage]` — named disks (Laravel `filesystems.php`).
///
/// Empty `disks` installs `local` (`./storage/app`, private) and `public`
/// (`./storage/app/public`, `/storage`). FTP/SFTP/S3 are registered with
/// [`crate::Storage::extend`], not built-in protocol crates.
///
/// ```toml
/// [storage]
/// default = "local"
///
/// [storage.disks.local]
/// driver = "local"
/// root = "./storage/app"
/// url = "/storage/private"
/// visibility = "private"
///
/// [storage.disks.public]
/// driver = "local"
/// root = "./storage/app/public"
/// url = "/storage"
/// visibility = "public"
///
/// [storage.links]
/// "public/storage" = "storage/app/public"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct StorageSection {
    #[serde(default = "default_storage_disk")]
    pub default: String,
    #[serde(default)]
    pub disks: BTreeMap<String, DiskConfig>,
    #[serde(default)]
    pub links: BTreeMap<String, String>,
}

impl Default for StorageSection {
    fn default() -> Self {
        Self {
            default: default_storage_disk(),
            disks: BTreeMap::new(),
            links: BTreeMap::new(),
        }
    }
}

fn default_storage_disk() -> String {
    "local".into()
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct DiskConfig {
    #[serde(default = "default_disk_driver")]
    pub driver: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub visibility: String,
    /// Source disk name for `scoped` / `readonly` wrappers.
    #[serde(default)]
    pub disk: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub bucket: String,
    #[serde(default)]
    pub endpoint: String,
}

fn default_disk_driver() -> String {
    "local".into()
}

/// `[database]` — Toasty 连接与开发期 schema。
///
/// ```toml
/// [database]
/// enabled = true
/// driver = "sqlite"   # sqlite | mysql | postgresql | custom
/// url = "sqlite:./storage/namix.db"   # 可空：按 driver + 分项拼
/// # host / port / name / username / password  — url 为空时用
/// push_schema = true
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSection {
    /// `false` 时 Boot 不连库（默认；即使编进了对应 feature）。
    #[serde(default)]
    pub enabled: bool,
    /// 驱动提示（文档/脚手架）；真实连接以 `url` / `DATABASE_URL` 为准。
    #[serde(default = "default_db_driver")]
    pub driver: String,
    /// 连接 URL。`DATABASE_URL` 环境变量优先；若为空则按 driver + 分项生成。
    /// 例：`sqlite:./storage/namix.db` / `postgresql://…` / `mysql://…`
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    /// 库名（mysql/postgresql）
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// 开发：启动时 `push_schema`；生产请改 `false` 并走 migration。
    #[serde(default = "default_true")]
    pub push_schema: bool,
}

impl Default for DatabaseSection {
    fn default() -> Self {
        Self {
            enabled: false,
            driver: default_db_driver(),
            url: String::new(),
            host: None,
            port: None,
            name: None,
            username: None,
            password: None,
            push_schema: true,
        }
    }
}

fn default_db_driver() -> String {
    "sqlite".into()
}

fn default_true() -> bool {
    true
}

impl DatabaseSection {
    /// `DATABASE_URL` → 非空 `url` → 按 driver/分项拼默认。
    pub fn resolved_url(&self) -> String {
        if let Ok(u) = std::env::var("DATABASE_URL")
            && !u.trim().is_empty()
        {
            return u;
        }
        if !self.url.trim().is_empty() {
            return self.url.clone();
        }
        self.url_from_driver()
    }

    fn url_from_driver(&self) -> String {
        let driver = self.driver.trim().to_ascii_lowercase();
        match driver.as_str() {
            "sqlite" | "sqlite3" => "sqlite:./storage/namix.db".into(),
            "mysql" | "mariadb" => {
                let host = self.host.as_deref().unwrap_or("127.0.0.1");
                let port = self.port.unwrap_or(3306);
                let name = self.name.as_deref().unwrap_or("namix");
                let user = self.username.as_deref().unwrap_or("root");
                let pass = self.password.as_deref().unwrap_or("");
                if pass.is_empty() {
                    format!("mysql://{user}@{host}:{port}/{name}")
                } else {
                    format!("mysql://{user}:{pass}@{host}:{port}/{name}")
                }
            }
            "postgresql" | "postgres" | "pg" => {
                let host = self.host.as_deref().unwrap_or("127.0.0.1");
                let port = self.port.unwrap_or(5432);
                let name = self.name.as_deref().unwrap_or("namix");
                let user = self.username.as_deref().unwrap_or("postgres");
                let pass = self.password.as_deref().unwrap_or("");
                if pass.is_empty() {
                    format!("postgresql://{user}@{host}:{port}/{name}")
                } else {
                    format!("postgresql://{user}:{pass}@{host}:{port}/{name}")
                }
            }
            // custom / 其它：必须显式写 url 或 DATABASE_URL
            _ => {
                eprintln!(
                    "[namix] database.driver={driver:?} 且 url 为空 — 回退 sqlite；请设置 url 或 DATABASE_URL"
                );
                "sqlite:./storage/namix.db".into()
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub hosts: Vec<String>,
    /// 推荐：只写端口，IP 由 lan / CLI `-h` 决定
    pub port: Option<u16>,
    /// 兼容旧写法：`127.0.0.1:3000`
    pub bind: Option<String>,
    /// `true` = 本地自签 HTTPS；或写 `"127.0.0.1:3443"` 兼容旧配置
    #[serde(default, deserialize_with = "de_https")]
    pub https: HttpsConfig,
    pub https_port: Option<u16>,
    #[serde(default)]
    pub http3: bool,
    #[serde(default)]
    pub lan: bool,
    #[serde(default)]
    pub tls_hosts: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub enum HttpsConfig {
    #[default]
    Off,
    On,
    Addr(String),
}

fn de_https<'de, D>(deserializer: D) -> Result<HttpsConfig, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Bool(bool),
        Str(String),
    }
    Ok(match Option::<Raw>::deserialize(deserializer)? {
        None => HttpsConfig::Off,
        Some(Raw::Bool(false)) => HttpsConfig::Off,
        Some(Raw::Bool(true)) => HttpsConfig::On,
        Some(Raw::Str(s)) if s.is_empty() || s == "false" => HttpsConfig::Off,
        Some(Raw::Str(s)) if s == "true" => HttpsConfig::On,
        Some(Raw::Str(s)) => HttpsConfig::Addr(s),
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeaturesSection {
    /// `src/models/` — Toasty 实体与查询助手
    #[serde(default)]
    pub models: bool,
    /// `src/services/` — 写库与领域服务
    #[serde(default)]
    pub services: bool,
    /// `src/validators/` — 表单验证器
    #[serde(default)]
    pub validators: bool,
    /// `src/requests/` — 请求 DTO 目录（可选分层）
    #[serde(default)]
    pub requests: bool,
    /// `src/views/` — React 页面（`req.view`）；需 Cargo feature `pages`
    #[serde(default)]
    pub pages: bool,
    /// `src/events/` — 领域事件类型
    #[serde(default)]
    pub events: bool,
    /// `src/listeners/` — 事件监听器
    #[serde(default)]
    pub listeners: bool,
    /// `src/seeders/` — 种子数据
    #[serde(default)]
    pub seeders: bool,
    /// `#[server]` 传输密封：`true`=生产整包加密；`false`=开发明文 JSON。
    /// 也可用环境变量 `NAMIX_ACTION_SEAL=0|1` 覆盖。
    #[serde(default = "default_true")]
    pub action_seal: bool,
}

impl Default for FeaturesSection {
    fn default() -> Self {
        Self {
            models: false,
            services: false,
            validators: false,
            requests: false,
            pages: false,
            events: false,
            listeners: false,
            seeders: false,
            action_seal: true,
        }
    }
}

impl NamixToml {
    pub fn parse(raw: &str) -> Self {
        Self::try_parse(raw).unwrap_or_else(|e| panic!("namix.toml 解析失败: {e}"))
    }

    pub fn try_parse(raw: &str) -> Result<Self, ConfigError> {
        toml::from_str(raw).map_err(|source| ConfigError::Parse { source })
    }

    pub fn app(&self, name: &str) -> &AppConfig {
        self.apps.get(name).unwrap_or_else(|| {
            panic!("namix.toml 缺少 [apps.{name}] 配置");
        })
    }

    /// Validate configuration deterministically at process start.  This keeps
    /// production-only secrets and TLS requirements from failing halfway
    /// through a request.
    pub fn validate(&self, app_name: &str) -> Result<(), ConfigError> {
        let environment = self.resolved_environment();
        self.validate_in_environment(app_name, &environment)
    }

    fn validate_in_environment(
        &self,
        app_name: &str,
        environment: &str,
    ) -> Result<(), ConfigError> {
        let mut messages = Vec::new();
        let Some(app) = self.apps.get(app_name) else {
            return Err(ConfigError::one(format!(
                "namix.toml missing [apps.{app_name}]"
            )));
        };

        if app.port == Some(0) || app.https_port == Some(0) {
            messages.push("listener ports must be between 1 and 65535".into());
        }
        if app.port.is_some() && app.bind.is_some() {
            messages.push("configure either apps.*.port or apps.*.bind, not both".into());
        }
        if let Some(bind) = app.bind.as_deref() {
            match bind.parse::<SocketAddr>() {
                Ok(address) if address.port() > 0 => {}
                _ => messages.push(
                    "apps.*.bind must be a numeric socket address with a non-zero port".into(),
                ),
            }
        }
        if let HttpsConfig::Addr(address) = &app.https {
            match address.parse::<SocketAddr>() {
                Ok(address) if address.port() > 0 => {}
                _ => messages.push(
                    "apps.*.https address must be a numeric socket address with a non-zero port"
                        .into(),
                ),
            }
        }
        if app.port.is_none() && app.bind.is_none() && matches!(app.https, HttpsConfig::Off) {
            messages.push("application must configure an HTTP or HTTPS listener".into());
        }
        if app.hosts.iter().any(|host| host.trim().is_empty())
            || app.tls_hosts.iter().any(|host| host.trim().is_empty())
        {
            messages.push("hosts and tls_hosts must not contain empty values".into());
        }
        if let Err(error) = crate::TrustedProxies::new(&self.security.trusted_proxies) {
            messages.push(format!("security.trusted_proxies: {error}"));
        }
        if !matches!(environment, "development" | "test" | "production") {
            messages.push(
                "resolved environment (NAMIX_ENV or security.environment) must be development, \
                 test, or production"
                    .into(),
            );
        }
        if self.security.rate_limit.enabled && self.security.rate_limit.window_seconds == 0 {
            messages.push("security.rate_limit.window_seconds must be greater than 0".into());
        }
        if self.database.enabled
            && !matches!(
                self.database.driver.to_ascii_lowercase().as_str(),
                "sqlite"
                    | "sqlite3"
                    | "mysql"
                    | "mariadb"
                    | "postgresql"
                    | "postgres"
                    | "pg"
                    | "turso"
                    | "dynamodb"
                    | "custom"
            )
        {
            messages.push("database.driver is not supported".into());
        }
        if self.database.enabled
            && matches!(
                self.database.driver.trim().to_ascii_lowercase().as_str(),
                "custom" | "turso" | "dynamodb"
            )
            && self.database.url.trim().is_empty()
            && std::env::var("DATABASE_URL")
                .ok()
                .is_none_or(|url| url.trim().is_empty())
        {
            messages.push(
                "database.driver=custom/turso/dynamodb requires database.url or DATABASE_URL"
                    .into(),
            );
        }
        if self.database.enabled && !database_driver_is_compiled(&self.database.driver) {
            messages.push(format!(
                "database.driver={} is not compiled into this application; enable the matching namix Cargo feature",
                self.database.driver.trim()
            ));
        }

        let session_driver = self.session.driver.trim().to_ascii_lowercase();
        if !matches!(session_driver.as_str(), "memory" | "file" | "redis" | "") {
            messages.push("session.driver must be memory, file, or redis".into());
        }
        if session_driver == "file" && self.session.path.trim().is_empty() {
            messages.push("session.driver=file requires session.path".into());
        }
        if self.session.lifetime_secs == 0 {
            messages.push("session.lifetime_secs must be greater than 0".into());
        }
        if self.session.jwt_lifetime_secs == 0 {
            messages.push("session.jwt_lifetime_secs must be greater than 0".into());
        }

        if environment == "production" {
            if matches!(app.https, HttpsConfig::Off) && !self.security.tls_terminated_by_proxy {
                messages.push(
                    "production requires HTTPS or security.tls_terminated_by_proxy = true".into(),
                );
            }
            if self.security.tls_terminated_by_proxy && !app_is_loopback_only(app) {
                messages.push(
                    "production with security.tls_terminated_by_proxy=true must bind only to \
                     loopback; expose it through the trusted TLS proxy"
                        .into(),
                );
            }
            if self.security.tls_terminated_by_proxy && self.security.trusted_proxies.is_empty() {
                messages.push(
                    "production TLS proxy mode requires security.trusted_proxies so client IP headers can be validated"
                        .into(),
                );
            }
            if !self.security.csrf {
                messages.push("production requires security.csrf = true".into());
            }
            if !self.features.action_seal {
                messages.push("production requires features.action_seal = true".into());
            }
            if self.database.enabled && self.database.push_schema {
                messages.push("production requires database.push_schema = false".into());
            }
            let secret = self
                .security
                .session_secret
                .clone()
                .or_else(|| std::env::var("NAMIX_SESSION_SECRET").ok())
                .unwrap_or_default();
            if secret.trim().is_empty() {
                messages.push(
                    "production requires security.session_secret or NAMIX_SESSION_SECRET".into(),
                );
            } else if secret.trim().len() < 32 {
                messages
                    .push("production session secret must contain at least 32 characters".into());
            }
            // Memory sessions cannot survive process overlap. Operators who
            // intentionally accept a cold cut may set NAMIX_ALLOW_MEMORY_SESSIONS=1.
            if matches!(session_driver.as_str(), "memory" | "") && !allow_memory_sessions_override()
            {
                messages.push(
                    "production requires session.driver=file or redis for overlap-safe \
                     releases; memory is single-process only (set \
                     NAMIX_ALLOW_MEMORY_SESSIONS=1 to acknowledge a maintenance-window cut)"
                        .into(),
                );
            }
            if self.sms.log_otp {
                messages.push("production requires sms.log_otp = false".into());
            }
        }

        if messages.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Validation { messages })
        }
    }

    pub fn is_production(&self) -> bool {
        self.resolved_environment() == "production"
    }

    fn resolved_environment(&self) -> String {
        std::env::var("NAMIX_ENV")
            .ok()
            .unwrap_or_else(|| self.security.environment.clone())
            .trim()
            .to_ascii_lowercase()
    }

    /// True when the configured session driver can be shared by overlapping
    /// processes (rolling `nx update`).
    pub fn session_is_shared(&self) -> bool {
        crate::session::driver_is_shared(&self.session.driver)
    }

    /// 按应用名组装 Server（可再叠加 CLI `-p` / `-h` / `--https`）。
    pub fn server_for(&self, app_name: &str) -> Server {
        let app = self.app(app_name);
        println!("namix app `{app_name}` hosts={:?}", app.hosts);

        let lan = app.lan;
        let ip: IpAddr = if lan {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };

        let http_port = app.port.or_else(|| {
            app.bind
                .as_ref()
                .and_then(|b| b.rsplit_once(':')?.1.parse().ok())
        });

        let mut server = if let Some(port) = app.port {
            Server::new().bind(SocketAddr::new(ip, port).to_string())
        } else if let Some(bind) = &app.bind {
            let mut s = Server::new().bind(bind);
            if lan {
                s = s.lan(true);
            }
            s
        } else {
            let mut s = Server::new();
            if lan {
                s = s.lan(true);
            }
            s
        };

        let hosts = tls_host_list(app);
        match &app.https {
            HttpsConfig::Off => {}
            HttpsConfig::On => {
                let https_port = app
                    .https_port
                    .unwrap_or_else(|| http_port.map(|p| p.saturating_add(443)).unwrap_or(3443));
                server = server
                    .local_https(true, https_port, &hosts)
                    .http3(app.http3);
                if lan {
                    server = server.lan(true);
                }
            }
            HttpsConfig::Addr(addr) => {
                server = server.https(addr).tls_self_signed(&hosts).http3(app.http3);
                if lan {
                    server = server.lan(true);
                }
            }
        }

        server
    }
}

fn app_is_loopback_only(app: &AppConfig) -> bool {
    if app.lan {
        return false;
    }
    let Some(bind) = app
        .bind
        .as_deref()
        .map(str::trim)
        .filter(|bind| !bind.is_empty())
    else {
        // A port-only config binds to loopback while `lan=false`.
        return true;
    };
    if let Ok(address) = bind.parse::<SocketAddr>() {
        return address.ip().is_loopback();
    }
    let host = bind.rsplit_once(':').map(|(host, _)| host).unwrap_or(bind);
    matches!(
        host.trim_matches(['[', ']']),
        "localhost" | "127.0.0.1" | "::1"
    )
}

fn allow_memory_sessions_override() -> bool {
    matches!(
        std::env::var("NAMIX_ALLOW_MEMORY_SESSIONS")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn database_driver_is_compiled(driver: &str) -> bool {
    match driver.trim().to_ascii_lowercase().as_str() {
        "sqlite" | "sqlite3" => cfg!(feature = "sqlite"),
        "mysql" | "mariadb" => cfg!(feature = "mysql"),
        "postgresql" | "postgres" | "pg" => cfg!(feature = "postgresql"),
        "turso" => cfg!(feature = "turso"),
        "dynamodb" => cfg!(feature = "dynamodb"),
        // Custom connectors are supplied by the application.
        "custom" => true,
        _ => false,
    }
}

fn tls_host_list(app: &AppConfig) -> Vec<&str> {
    if !app.tls_hosts.is_empty() {
        return app.tls_hosts.iter().map(String::as_str).collect();
    }
    if !app.hosts.is_empty() {
        return app.hosts.iter().map(String::as_str).collect();
    }
    vec!["localhost", "127.0.0.1"]
}

#[cfg(test)]
mod tests {
    use super::NamixToml;

    #[test]
    fn production_requires_tls_secret_and_migration_discipline() {
        let cfg = NamixToml::try_parse(
            r#"
            [database]
            enabled = true
            push_schema = true
            [features]
            action_seal = false
            [security]
            environment = "production"
            csrf = false
            [apps.main]
            port = 3000
            https = false
            "#,
        )
        .unwrap();
        let error = cfg.validate("main").unwrap_err();
        let text = error.to_string();
        assert!(text.contains("HTTPS"));
        assert!(text.contains("session_secret"));
        assert!(text.contains("push_schema"));
    }

    #[test]
    fn production_allows_a_trusted_tls_terminating_proxy() {
        let cfg = NamixToml::try_parse(
            r#"
            [database]
            enabled = false
            push_schema = false
            [features]
            action_seal = true
            [session]
            driver = "file"
            path = "./storage/sessions"
            [security]
            environment = "production"
            csrf = true
            tls_terminated_by_proxy = true
            trusted_proxies = ["127.0.0.1"]
            session_secret = "a-very-long-production-session-secret"
            [apps.main]
            bind = "127.0.0.1:3000"
            https = false
            "#,
        )
        .unwrap();
        assert!(cfg.validate("main").is_ok());
        assert!(cfg.session_is_shared());
    }

    #[test]
    fn production_rejects_memory_session_driver_by_default() {
        let cfg = NamixToml::try_parse(
            r#"
            [database]
            enabled = false
            push_schema = false
            [features]
            action_seal = true
            [session]
            driver = "memory"
            [security]
            environment = "production"
            csrf = true
            tls_terminated_by_proxy = true
            session_secret = "a-very-long-production-session-secret"
            [apps.main]
            bind = "127.0.0.1:3000"
            https = false
            "#,
        )
        .unwrap();
        let text = cfg.validate("main").unwrap_err().to_string();
        assert!(text.contains("session.driver"));
    }

    #[test]
    fn parse_error_retains_toml_diagnostic_as_source() {
        let error = NamixToml::try_parse("[apps.main").unwrap_err();
        assert!(matches!(error, super::ConfigError::Parse { .. }));
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn development_config_has_secure_defaults_without_a_secret() {
        let cfg = NamixToml::try_parse(
            r#"
            [apps.main]
            port = 3000
            "#,
        )
        .unwrap();
        assert!(cfg.validate("main").is_ok());
    }

    #[test]
    fn invalid_environment_override_is_rejected() {
        let cfg = NamixToml::try_parse(
            r#"
            [security]
            environment = "development"
            [apps.main]
            port = 3000
            "#,
        )
        .unwrap();
        let error = cfg
            .validate_in_environment("main", "prodution")
            .unwrap_err()
            .to_string();
        assert!(error.contains("resolved environment"));
    }

    #[test]
    fn environment_override_to_production_enforces_production_rules() {
        let cfg = NamixToml::try_parse(
            r#"
            [security]
            environment = "development"
            csrf = false
            [features]
            action_seal = false
            [apps.main]
            port = 3000
            https = false
            "#,
        )
        .unwrap();
        let error = cfg
            .validate_in_environment("main", "production")
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTPS"));
        assert!(error.contains("csrf"));
        assert!(error.contains("action_seal"));
    }

    #[test]
    fn tls_terminating_proxy_requires_a_loopback_bind() {
        let cfg = NamixToml::try_parse(
            r#"
            [features]
            action_seal = true
            [session]
            driver = "file"
            [security]
            csrf = true
            tls_terminated_by_proxy = true
            session_secret = "a-very-long-production-session-secret"
            [apps.main]
            bind = "0.0.0.0:3000"
            https = false
            "#,
        )
        .unwrap();
        let error = cfg
            .validate_in_environment("main", "production")
            .unwrap_err()
            .to_string();
        assert!(error.contains("loopback"));
    }

    #[test]
    fn reserved_database_session_driver_is_rejected_at_validation() {
        let cfg = NamixToml::try_parse(
            r#"
            [session]
            driver = "database"
            [apps.main]
            port = 3000
            "#,
        )
        .unwrap();
        assert!(
            cfg.validate_in_environment("main", "development")
                .unwrap_err()
                .to_string()
                .contains("session.driver")
        );
    }

    #[test]
    fn listener_validation_rejects_ambiguous_or_invalid_bindings() {
        let both = NamixToml::try_parse(
            r#"
            [apps.main]
            port = 3000
            bind = "127.0.0.1:3001"
            "#,
        )
        .unwrap();
        assert!(
            both.validate("main")
                .unwrap_err()
                .to_string()
                .contains("not both")
        );

        let invalid = NamixToml::try_parse(
            r#"
            [apps.main]
            bind = "localhost:not-a-port"
            "#,
        )
        .unwrap();
        assert!(
            invalid
                .validate("main")
                .unwrap_err()
                .to_string()
                .contains("numeric socket address")
        );
    }

    #[test]
    fn explicit_bind_address_is_preserved_by_server_builder() {
        let cfg = NamixToml::try_parse(
            r#"
            [apps.main]
            bind = "127.0.0.2:4321"
            "#,
        )
        .unwrap();
        cfg.validate("main").unwrap();
        assert_eq!(
            cfg.server_for("main").http_addr().unwrap().to_string(),
            "127.0.0.2:4321"
        );
    }

    #[test]
    fn trusted_proxy_entries_are_validated_at_startup() {
        let cfg = NamixToml::try_parse(
            r#"
            [security]
            trusted_proxies = ["127.0.0.1/64"]
            [apps.main]
            port = 3000
            "#,
        )
        .unwrap();
        assert!(
            cfg.validate("main")
                .unwrap_err()
                .to_string()
                .contains("trusted_proxies")
        );
    }
}
