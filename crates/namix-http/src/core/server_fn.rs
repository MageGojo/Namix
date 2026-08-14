//! Server Actions（≈ Leptos `#[server]`）。
//!
//! ## 开关
//! - `action_seal = false`（开发）：明文 JSON 包络，Network 可见字段
//! - `action_seal = true`（生产）：整包 `octet-stream` + ECDH
//!
//! ## 零预备请求
//! - 公钥写入 `storage/action_seal.key`，编译进 WASM（无 `GET /api/k`）
//! - 客户端单次 `POST /api/a`，包络 `{ t: 动作token, i: 入参, ts }`（无领票）
//! - AES 由客户端临时私钥 × 应用公钥 ECDH 得出，从不下发

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use serde::Serialize;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};

use super::content_type::ContentType;
use super::error::AppError;
use super::rate_limit::{RateLimitPolicy, RateLimiter};
use super::request::Request;
use super::response::Response;
use super::routing::Router;
use crate::core::routing::Route;
use http::StatusCode;

pub const SEAL_MAGIC: &[u8; 3] = b"NX\x01";
pub const SEAL_FIELD: &str = "_nx";

const ENVELOPE_SKEW_SECS: u64 = 120;
const HKDF_SALT: &[u8] = b"namix-action-v1";
const HKDF_INFO: &[u8] = b"aes-256-gcm";
const KEY_FILE: &str = "storage/action_seal.key";

type BoxRespFut = Pin<Box<dyn Future<Output = Response> + Send>>;

pub struct ServerFn {
    pub name: &'static str,
    /// 混淆用短 token（非可读动作名）。
    pub token: &'static str,
    pub seal: &'static [&'static str],
    pub call: fn(Request) -> BoxRespFut,
}

inventory::collect!(ServerFn);

static ACTION_SEAL: OnceLock<bool> = OnceLock::new();
static APP_X25519: OnceLock<([u8; 32], [u8; 32])> = OnceLock::new();
static ACTION_RATE_LIMITS: OnceLock<ActionRateLimits> = OnceLock::new();

/// Category budgets for generated `#[server]` endpoints.
#[derive(Clone)]
pub struct ActionRateLimits {
    pub limiter: RateLimiter,
    pub login: RateLimitPolicy,
    pub registration: RateLimitPolicy,
    pub action: RateLimitPolicy,
}

impl ActionRateLimits {
    pub fn new(
        limiter: RateLimiter,
        login: RateLimitPolicy,
        registration: RateLimitPolicy,
        action: RateLimitPolicy,
    ) -> Self {
        Self {
            limiter,
            login,
            registration,
            action,
        }
    }

    fn policy_for(&self, name: &str) -> (&str, &RateLimitPolicy) {
        match name {
            "login" => ("login", &self.login),
            "register" | "registration" => ("registration", &self.registration),
            _ => ("action", &self.action),
        }
    }
}

/// Install action budgets before the server begins accepting requests.
pub fn configure_rate_limits(limits: ActionRateLimits) {
    let _ = ACTION_RATE_LIMITS.set(limits);
}

/// `seal=true` → 生产整包加密；`false` → 开发明文 JSON。
pub fn configure(_app: impl Into<String>, seal: bool) {
    let _ = ACTION_SEAL.set(seal);
    let _ = APP_X25519.get_or_init(load_or_create_keypair);
}

fn load_or_create_keypair() -> ([u8; 32], [u8; 32]) {
    let path = Path::new(KEY_FILE);
    if let Ok(bytes) = fs::read(path) {
        if bytes.len() == 64 {
            let mut secret = [0u8; 32];
            let mut public = [0u8; 32];
            secret.copy_from_slice(&bytes[..32]);
            public.copy_from_slice(&bytes[32..]);
            return (secret, public);
        }
        if bytes.len() == 32 {
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            let sk = StaticSecret::from(secret);
            let pk = PublicKey::from(&sk);
            let public = *pk.as_bytes();
            write_keypair(path, &secret, &public);
            return (secret, public);
        }
    }

    let sk = StaticSecret::random_from_rng(rand::thread_rng());
    let pk = PublicKey::from(&sk);
    let secret = sk.to_bytes();
    let public = *pk.as_bytes();
    write_keypair(path, &secret, &public);
    (secret, public)
}

fn write_keypair(path: &Path, secret: &[u8; 32], public: &[u8; 32]) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(secret);
    buf[32..].copy_from_slice(public);
    if let Err(e) = fs::write(path, buf) {
        eprintln!("[namix] warn: cannot persist {KEY_FILE}: {e}");
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
            eprintln!("[namix] warn: cannot chmod 0600 {KEY_FILE}: {error}");
        }
    }
}

pub fn action_seal_enabled() -> bool {
    // 环境变量优先：NAMIX_ACTION_SEAL=0|1|true|false
    if let Ok(v) = std::env::var("NAMIX_ACTION_SEAL") {
        return matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on");
    }
    *ACTION_SEAL.get().unwrap_or(&true)
}

fn app_secret() -> [u8; 32] {
    APP_X25519.get_or_init(load_or_create_keypair).0
}

/// 与宏端一致的动作 token（FNV-1a 64 → 8 hex）。
pub fn action_token(name: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in name.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")[..8].to_string()
}

#[derive(Clone)]
pub struct ActionSealKey(pub [u8; 32]);

#[derive(Clone)]
struct ActionServerSecret(pub [u8; 32]);

pub fn routes() -> Router {
    Router::new().merge(
        Route::post("/api/a", dispatch_action)
            .name("__namix.api.a")
            .register(),
    )
}

async fn dispatch_action(mut req: Request) -> Response {
    let sealed = req.body().len() >= 3 && &req.body()[..3] == SEAL_MAGIC;

    if action_seal_enabled() {
        if !sealed {
            return action_error(
                StatusCode::BAD_REQUEST,
                "seal on: send application/octet-stream",
            );
        }
        req.set(ActionServerSecret(app_secret()));
        if let Err(resp) = materialize_sealed_body(&mut req) {
            return resp;
        }
    } else if sealed {
        req.set(ActionServerSecret(app_secret()));
        if let Err(resp) = materialize_sealed_body(&mut req) {
            return resp;
        }
    }

    let (tok, input) = match peel_envelope(&req) {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    req.set_body(input);
    req.set_header("content-type", "application/json; charset=utf-8");

    let Some(item) = find_by_token(&tok) else {
        return action_error(StatusCode::NOT_FOUND, "unknown action");
    };
    if let Some(limits) = ACTION_RATE_LIMITS.get() {
        let (namespace, policy) = limits.policy_for(item.name);
        if let Err(error) = limits.limiter.check(&req, namespace, policy) {
            return error.into_action_response();
        }
    }
    (item.call)(req).await
}

#[allow(clippy::result_large_err)]
fn peel_envelope(req: &Request) -> Result<(String, Vec<u8>), Response> {
    let v: serde_json::Value = serde_json::from_slice(req.body()).map_err(|error| {
        tracing::debug!(error = %error, "action envelope json decode failed");
        action_error(StatusCode::BAD_REQUEST, "invalid action envelope")
    })?;
    let tok = v
        .get("t")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| action_error(StatusCode::BAD_REQUEST, "envelope missing t"))?
        .to_string();
    let Some(ts) = v.get("ts").and_then(|x| x.as_u64()) else {
        return Err(action_error(StatusCode::BAD_REQUEST, "envelope missing ts"));
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if now.abs_diff(ts) > ENVELOPE_SKEW_SECS {
        return Err(action_error(StatusCode::GONE, "envelope expired"));
    }
    let input = v.get("i").cloned().unwrap_or(serde_json::json!({}));
    let bytes = serde_json::to_vec(&input)
        .map_err(|_| action_error(StatusCode::BAD_REQUEST, "invalid action envelope"))?;
    Ok((tok, bytes))
}

fn find_by_token(tok: &str) -> Option<&'static ServerFn> {
    inventory::iter::<ServerFn>
        .into_iter()
        .find(|f| f.token == tok)
}

fn derive_aes_key(server_secret: &[u8; 32], client_pub: &[u8; 32]) -> Result<[u8; 32], String> {
    let secret = StaticSecret::from(*server_secret);
    let their = PublicKey::from(*client_pub);
    let shared = secret.diffie_hellman(&their);
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), shared.as_bytes());
    let mut aes_key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut aes_key)
        .map_err(|_| "HKDF expand failed".to_string())?;
    Ok(aes_key)
}

fn aes_decrypt(key: &[u8; 32], nonce: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(nonce);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "AES-GCM decrypt failed".into())
}

#[allow(clippy::result_large_err)]
pub fn materialize_sealed_body(req: &mut Request) -> Result<(), Response> {
    if req.body().is_empty() {
        return Ok(());
    }

    let body = req.body().to_vec();
    if body.len() >= 3 && &body[..3] == SEAL_MAGIC {
        let server = req.get::<ActionServerSecret>().ok_or_else(|| {
            action_error(
                StatusCode::BAD_REQUEST,
                "sealed body requires action context",
            )
        })?;
        let (key, plain) = open_sealed_blob(&body, &server.0)
            .map_err(|e| action_error(StatusCode::BAD_REQUEST, format!("unseal: {e}")))?;
        serde_json::from_slice::<serde_json::Value>(&plain)
            .map_err(|e| action_error(StatusCode::BAD_REQUEST, format!("sealed json: {e}")))?;
        req.set(ActionSealKey(key));
        req.set_body(plain);
        req.set_header("content-type", "application/json; charset=utf-8");
        return Ok(());
    }

    if action_seal_enabled() {
        let ct = req.header_or("content-type", "");
        if ct.contains("json") {
            return Err(action_error(
                StatusCode::BAD_REQUEST,
                "seal on: send application/octet-stream (not plaintext JSON)",
            ));
        }
    }

    Ok(())
}

fn open_sealed_blob(blob: &[u8], server_secret: &[u8; 32]) -> Result<([u8; 32], Vec<u8>), String> {
    if blob.len() < 3 + 32 + 12 + 16 {
        return Err("ciphertext too short".into());
    }
    if &blob[..3] != SEAL_MAGIC {
        return Err("bad magic".into());
    }
    let mut client_pub = [0u8; 32];
    client_pub.copy_from_slice(&blob[3..35]);
    let nonce = &blob[35..47];
    let ct = &blob[47..];
    let key = derive_aes_key(server_secret, &client_pub)?;
    let plain = aes_decrypt(&key, nonce, ct)?;
    Ok((key, plain))
}

pub fn expand_input_map(req: &Request) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();

    if let Some(qs) = req.query_string() {
        parse_form_like(qs, &mut map);
    }

    if req.body().is_empty() {
        return Ok(map);
    }

    let body = req.body();
    if body.len() >= 3 && &body[..3] == SEAL_MAGIC {
        let server = req
            .get::<ActionServerSecret>()
            .ok_or_else(|| "sealed body requires action context".to_string())?;
        let (_key, plain) = open_sealed_blob(body, &server.0)?;
        if let Ok(serde_json::Value::Object(obj)) = serde_json::from_slice(&plain) {
            for (k, v) in obj {
                map.insert(k, json_scalar(&v));
            }
        }
        return Ok(map);
    }

    let ct = req.header_or("content-type", "");
    if crate::core::upload::is_multipart(ct) {
        let bag = req.form_bag();
        for (k, v) in &bag.fields {
            map.insert(k.clone(), v.clone());
        }
        for (k, file) in &bag.files {
            map.entry(k.clone())
                .or_insert_with(|| file.filename.clone());
        }
        return Ok(map);
    }

    if (ct.contains("json") || ct.is_empty())
        && let Ok(serde_json::Value::Object(obj)) =
            serde_json::from_slice::<serde_json::Value>(body)
    {
        for (k, v) in obj {
            map.insert(k, json_scalar(&v));
        }
        return Ok(map);
    }

    parse_form_like(req.body_str(), &mut map);
    Ok(map)
}

fn parse_form_like(raw: &str, map: &mut HashMap<String, String>) {
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        map.insert(
            crate::core::request::query_decode_pub(k),
            crate::core::request::query_decode_pub(v),
        );
    }
}

fn json_scalar(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub struct ActionOk<T> {
    pub data: T,
    set_cookies: Vec<(String, String, crate::core::response::CookieOptions)>,
    clear_cookies: Vec<(String, crate::core::response::CookieOptions)>,
}

impl<T> ActionOk<T> {
    pub fn new(data: T) -> Self {
        Self {
            data,
            set_cookies: Vec::new(),
            clear_cookies: Vec::new(),
        }
    }

    pub fn with_cookie(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set_cookies.push((
            name.into(),
            value.into(),
            crate::core::response::CookieOptions::default(),
        ));
        self
    }

    pub fn with_cookie_options(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
        options: crate::core::response::CookieOptions,
    ) -> Self {
        self.set_cookies.push((name.into(), value.into(), options));
        self
    }

    pub fn with_clear_cookie(mut self, name: impl Into<String>) -> Self {
        self.clear_cookies
            .push((name.into(), crate::core::response::CookieOptions::default()));
        self
    }

    pub fn with_clear_cookie_options(
        mut self,
        name: impl Into<String>,
        options: crate::core::response::CookieOptions,
    ) -> Self {
        self.clear_cookies.push((name.into(), options));
        self
    }
}

impl<T: Serialize> ActionOk<T> {
    pub fn into_response(self) -> Response {
        let mut resp = match serde_json::to_string(&self.data) {
            Ok(body) => Response::new(StatusCode::OK, ContentType::Json, body),
            Err(e) => action_error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
        for (name, value, options) in self.set_cookies {
            resp = resp.with_cookie_options(&name, &value, options);
        }
        for (name, options) in self.clear_cookies {
            resp = resp.with_clear_cookie_options(&name, options);
        }
        resp
    }
}

pub trait IntoActionResponse {
    fn into_action_response(self) -> Response;
}

impl<T: Serialize> IntoActionResponse for ActionOk<T> {
    fn into_action_response(self) -> Response {
        self.into_response()
    }
}

impl<T: Serialize> IntoActionResponse for Result<ActionOk<T>, String> {
    fn into_action_response(self) -> Response {
        match self {
            Ok(ok) => ok.into_response(),
            Err(msg) => ActionError::message(msg).into_response(),
        }
    }
}

impl<T: Serialize> IntoActionResponse for Result<ActionOk<T>, ActionError> {
    fn into_action_response(self) -> Response {
        match self {
            Ok(ok) => ok.into_response(),
            Err(err) => err.into_response(),
        }
    }
}

impl<T: Serialize> IntoActionResponse for Result<ActionOk<T>, AppError> {
    fn into_action_response(self) -> Response {
        match self {
            Ok(ok) => ok.into_response(),
            Err(error) => error.into_action_response(),
        }
    }
}

impl<T: Serialize> IntoActionResponse
    for Result<ActionOk<T>, crate::core::validate::ValidationError>
{
    fn into_action_response(self) -> Response {
        match self {
            Ok(ok) => ok.into_response(),
            Err(err) => ActionError::from(err).into_response(),
        }
    }
}

impl<T: Serialize> IntoActionResponse for Result<T, String> {
    fn into_action_response(self) -> Response {
        match self {
            Ok(data) => ActionOk::new(data).into_response(),
            Err(msg) => ActionError::message(msg).into_response(),
        }
    }
}

impl<T: Serialize> IntoActionResponse for Result<T, ActionError> {
    fn into_action_response(self) -> Response {
        match self {
            Ok(data) => ActionOk::new(data).into_response(),
            Err(err) => err.into_response(),
        }
    }
}

impl<T: Serialize> IntoActionResponse for Result<T, AppError> {
    fn into_action_response(self) -> Response {
        match self {
            Ok(data) => ActionOk::new(data).into_response(),
            Err(error) => error.into_action_response(),
        }
    }
}

/// Server Action 业务/校验错误（422 JSON）。
///
/// ```json
/// { "error": "…", "message": "…", "errors": { "username": "已被占用" } }
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct ActionError {
    pub message: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub errors: HashMap<String, String>,
}

impl ActionError {
    pub fn message(msg: impl Into<String>) -> Self {
        let message = msg.into();
        let mut errors = HashMap::new();
        errors.insert("_".into(), message.clone());
        Self { message, errors }
    }

    pub fn with_field(mut self, field: impl Into<String>, msg: impl Into<String>) -> Self {
        self.errors.insert(field.into(), msg.into());
        self
    }

    pub fn field(field: impl Into<String>, msg: impl Into<String>) -> Self {
        let field = field.into();
        let msg = msg.into();
        Self {
            message: msg.clone(),
            errors: HashMap::from([(field, msg)]),
        }
    }

    pub fn into_response(self) -> Response {
        action_error_json(StatusCode::UNPROCESSABLE_ENTITY, &self)
    }
}

impl From<String> for ActionError {
    fn from(message: String) -> Self {
        Self::message(message)
    }
}

impl From<&str> for ActionError {
    fn from(message: &str) -> Self {
        Self::message(message)
    }
}

impl From<crate::core::validate::ValidationError> for ActionError {
    fn from(err: crate::core::validate::ValidationError) -> Self {
        let mut errors = HashMap::new();
        for (field, msgs) in err.errors() {
            if let Some(first) = msgs.first() {
                errors.insert(field.clone(), first.clone());
            }
        }
        let message = err.first().unwrap_or("validation.failed").to_string();
        if errors.is_empty() {
            errors.insert("_".into(), message.clone());
        }
        Self { message, errors }
    }
}

pub fn action_error(status: StatusCode, message: impl Into<String>) -> Response {
    action_error_json(status, &ActionError::message(message))
}

fn action_error_json(status: StatusCode, err: &ActionError) -> Response {
    let body = serde_json::json!({
        "error": err.message,
        "message": err.message,
        "errors": err.errors,
    })
    .to_string();
    Response::new(status, ContentType::Json, body)
}

#[allow(clippy::result_large_err)]
pub fn parse_json_body<T: serde::de::DeserializeOwned>(req: &Request) -> Result<T, Response> {
    match req.json::<T>() {
        Ok(v) => Ok(v),
        Err(e) => Err(action_error(StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub fn wants_html_navigation(req: &Request) -> bool {
    let accept = req.header("accept").unwrap_or("");
    if accept.contains("application/json") {
        return false;
    }
    let mode = req.header("sec-fetch-mode").unwrap_or("");
    mode.eq_ignore_ascii_case("navigate") || accept.contains("text/html") || accept.is_empty()
}

pub async fn finalize_action(wants_navigation: bool, resp: Response) -> Response {
    if !wants_navigation {
        return resp;
    }

    let (status, headers, body) = resp.into_status_headers_body().await;
    let redirect = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("redirect")
                .and_then(|x| x.as_str())
                .map(str::to_string)
        });

    let Some(location) = redirect else {
        return Response::from_parts(status, headers, body);
    };

    let mut out = Response::redirect_see_other(location);
    for value in headers.get_all(http::header::SET_COOKIE) {
        out.headers_mut()
            .append(http::header::SET_COOKIE, value.clone());
    }
    out
}

pub use inventory;

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::{HeaderMap, Method, Uri};

    use super::*;

    fn envelope_req(json: &str) -> Request {
        Request::new(
            Method::POST,
            Uri::from_static("/api/a"),
            HeaderMap::new(),
            Bytes::from(json.to_owned()),
        )
    }

    #[test]
    fn envelope_requires_timestamp() {
        let req = envelope_req(r#"{"t":"deadbeef","i":{}}"#);
        let err = peel_envelope(&req).unwrap_err();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn envelope_rejects_stale_timestamp() {
        let req = envelope_req(r#"{"t":"deadbeef","i":{},"ts":1}"#);
        let err = peel_envelope(&req).unwrap_err();
        assert_eq!(err.status(), StatusCode::GONE);
    }

    #[test]
    fn envelope_accepts_fresh_timestamp() {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let req = envelope_req(&format!(r#"{{"t":"deadbeef","i":{{"x":1}},"ts":{ts}}}"#));
        let (token, body) = peel_envelope(&req)
            .unwrap_or_else(|resp| panic!("expected fresh envelope, got {}", resp.status()));
        assert_eq!(token, "deadbeef");
        assert!(String::from_utf8(body).unwrap().contains("x"));
    }
}
