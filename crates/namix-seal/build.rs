//! 生成嵌入 WASM 的应用 X25519 公钥 + seal 开关。
//! 密钥文件与服务端共用：`app/storage/action_seal.key`（64B = secret||public）。

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use x25519_dalek::{PublicKey, StaticSecret};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let key_path = manifest.join("../../app/storage/action_seal.key");
    let toml_path = manifest.join("../../app/namix.toml");

    println!("cargo:rerun-if-changed={}", key_path.display());
    println!("cargo:rerun-if-changed={}", toml_path.display());
    println!("cargo:rerun-if-env-changed=NAMIX_ACTION_SEAL");

    let (_secret, public) = load_or_create_keypair(&key_path);
    let seal = read_seal_flag(&toml_path);

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let spk_lit = public
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        out.join("embed.rs"),
        format!("pub const SPK: [u8; 32] = [{spk_lit}];\npub const SEAL_ENABLED: bool = {seal};\n"),
    )
    .expect("write embed.rs");
}

fn load_or_create_keypair(path: &Path) -> ([u8; 32], [u8; 32]) {
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
    fs::write(path, buf).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

fn read_seal_flag(toml_path: &Path) -> bool {
    if let Ok(v) = env::var("NAMIX_ACTION_SEAL") {
        return matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on");
    }
    let Ok(raw) = fs::read_to_string(toml_path) else {
        return true;
    };
    // 极简解析：[features] 下 action_seal = true/false
    let mut in_features = false;
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_features = line == "[features]";
            continue;
        }
        if in_features && let Some(rest) = line.strip_prefix("action_seal") {
            let rest = rest.trim().trim_start_matches('=').trim();
            return !matches!(rest, "false" | "0" | "\"false\"");
        }
    }
    true
}
