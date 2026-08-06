//! Namix action 客户端密封逻辑（WASM）。
//!
//! 零预备请求：公钥嵌入本模块，包络 `{ t, i, ts }` 密封后单次 `POST /api/a`。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use js_sys::Uint8Array;
use rand::RngCore;
use sha2::Sha256;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestCredentials, RequestInit, RequestMode, Response};
use x25519_dalek::{PublicKey, StaticSecret};

include!(concat!(env!("OUT_DIR"), "/embed.rs"));

const MAGIC: [u8; 3] = [0x4e, 0x58, 0x01]; // NX1
const HKDF_SALT: &[u8] = b"namix-action-v1";
const HKDF_INFO: &[u8] = b"aes-256-gcm";

/// XOR 混淆的路径（避免明文躺在 JS bundle）。
fn decode_path(masked: &[u8], key: u8) -> String {
    masked.iter().map(|b| (b ^ key) as char).collect()
}

fn path_action() -> String {
    // "/api/a" XOR 0x5A
    decode_path(&[0x75, 0x3b, 0x2a, 0x33, 0x75, 0x3b], 0x5A)
}

fn seal_blob(spk: &[u8; 32], plain_json: &str) -> Result<Vec<u8>, String> {
    let client_secret = StaticSecret::random_from_rng(rand::thread_rng());
    let client_public = PublicKey::from(&client_secret);
    let server_public = PublicKey::from(*spk);
    let shared = client_secret.diffie_hellman(&server_public);

    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), shared.as_bytes());
    let mut aes_key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut aes_key)
        .map_err(|_| "hkdf".to_string())?;

    let cipher = Aes256Gcm::new_from_slice(&aes_key).map_err(|e| e.to_string())?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plain_json.as_bytes())
        .map_err(|_| "aes".to_string())?;

    let mut out = Vec::with_capacity(3 + 32 + 12 + ct.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(client_public.as_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

async fn fetch_bytes(url: &str, init: RequestInit) -> Result<(u16, String), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let req = Request::new_with_str_and_init(url, &init)?;
    let resp_val = JsFuture::from(window.fetch_with_request(&req)).await?;
    let resp: Response = resp_val.dyn_into()?;
    let status = resp.status();
    let text = JsFuture::from(resp.text()?).await?;
    Ok((status, text.as_string().unwrap_or_default()))
}

fn now_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

fn csrf_cookie() -> Option<String> {
    let document: web_sys::HtmlDocument = web_sys::window()?.document()?.dyn_into().ok()?;
    let raw = document.cookie().ok()?;
    raw.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == "namix_csrf" && !value.is_empty()).then(|| value.to_string())
    })
}

/// 单次 POST：包络 `{ t: tok, i: body, ts }` →（可选密封）→ `/api/a`。
#[wasm_bindgen]
pub async fn nx_call(tok: &str, body_json: &str) -> Result<String, JsValue> {
    let input: serde_json::Value = if body_json.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(body_json).map_err(|e| JsValue::from_str(&e.to_string()))?
    };
    let envelope = serde_json::json!({
        "t": tok,
        "i": input,
        "ts": now_secs(),
    });
    let plain = envelope.to_string();

    let post = RequestInit::new();
    post.set_method("POST");
    post.set_mode(RequestMode::SameOrigin);
    post.set_credentials(RequestCredentials::SameOrigin);

    let headers = web_sys::Headers::new()?;
    headers.set("accept", "application/json")?;
    if let Some(token) = csrf_cookie() {
        headers.set("x-csrf-token", &token)?;
    }

    if SEAL_ENABLED {
        let sealed = seal_blob(&SPK, &plain).map_err(|e| JsValue::from_str(&e))?;
        let u8 = Uint8Array::new_with_length(sealed.len() as u32);
        u8.copy_from(&sealed);
        headers.set("content-type", "application/octet-stream")?;
        post.set_headers(&headers);
        post.set_body(&u8);
    } else {
        headers.set("content-type", "application/json")?;
        post.set_headers(&headers);
        post.set_body(&JsValue::from_str(&plain));
    }

    let (status, text) = fetch_bytes(&path_action(), post).await?;
    if !(200..300).contains(&status) {
        // 原样抛 JSON 正文，供 useForm 解析 `{ error, errors }` 字段级错误
        let payload = if text.trim_start().starts_with('{') {
            text
        } else {
            serde_json::json!({ "error": text, "errors": { "_": text } }).to_string()
        };
        return Err(JsValue::from_str(&payload));
    }
    Ok(text)
}
