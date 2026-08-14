//! Tiny JSON dictionary i18n (`lang/zh-CN.json` nested objects, dotted keys).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::config::I18nSection;
use namix_http::install_error_translator;
use serde_json::Value;

#[derive(Clone, Debug, Default)]
struct Catalog {
    locale: String,
    messages: HashMap<String, String>,
}

static CATALOG: RwLock<Option<Catalog>> = RwLock::new(None);

pub fn init(cfg: &I18nSection) {
    let locale = if cfg.locale.trim().is_empty() {
        "zh-CN".into()
    } else {
        cfg.locale.trim().to_string()
    };
    let dir = if cfg.path.trim().is_empty() {
        PathBuf::from("./lang")
    } else {
        PathBuf::from(cfg.path.trim())
    };
    let path = dir.join(format!("{locale}.json"));
    let messages = load_file(&path).unwrap_or_default();
    *CATALOG.write().expect("i18n lock") = Some(Catalog { locale, messages });
    install_error_translator(trans_error);
}

pub fn locale() -> String {
    CATALOG
        .read()
        .ok()
        .and_then(|c| c.as_ref().map(|c| c.locale.clone()))
        .filter(|locale| !locale.is_empty())
        .unwrap_or_else(|| "zh-CN".into())
}

/// Look up `auth.failed`. Missing keys return the key itself.
pub fn trans(key: &str) -> String {
    CATALOG
        .read()
        .ok()
        .and_then(|c| c.as_ref().and_then(|c| c.messages.get(key).cloned()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| key.to_string())
}

/// Replace `:name` placeholders in a translated string.
pub fn trans_with(key: &str, params: &[(&str, &str)]) -> String {
    let mut out = trans(key);
    for (name, value) in params {
        out = out.replace(&format!(":{name}"), value);
    }
    out
}

/// Resolve a validation code: exact key, then `validation.{rule}` with `:attribute`.
/// `:attribute` prefers `attributes.{field}` when present.
pub fn trans_error(code: &str) -> String {
    let specific = trans(code);
    if specific != code {
        return specific;
    }
    let Some((attribute, rule)) = code.rsplit_once('.') else {
        return code.to_string();
    };
    let fallback_key = format!("validation.{rule}");
    let attr_key = format!("attributes.{attribute}");
    let attr_label = trans(&attr_key);
    let display = if attr_label != attr_key {
        attr_label
    } else {
        attribute.to_string()
    };
    let fallback = trans_with(&fallback_key, &[("attribute", display.as_str())]);
    if fallback != fallback_key {
        return fallback;
    }
    code.to_string()
}

fn load_file(path: &Path) -> Option<HashMap<String, String>> {
    let raw = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let mut out = HashMap::new();
    flatten("", &value, &mut out);
    Some(out)
}

fn flatten(prefix: &str, value: &Value, out: &mut HashMap<String, String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&next, child, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        other if !prefix.is_empty() => {
            out.insert(prefix.to_string(), other.to_string());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static I18N_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_catalog(messages: HashMap<String, String>, f: impl FnOnce()) {
        let _lock = I18N_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *CATALOG.write().expect("i18n lock") = Some(Catalog {
            locale: "zh-CN".into(),
            messages,
        });
        f();
        *CATALOG.write().expect("i18n lock") = None;
    }

    #[test]
    fn flattens_nested_json() {
        let value = serde_json::json!({
            "auth": { "failed": "用户名或密码不正确" },
            "plain": "ok"
        });
        let mut out = HashMap::new();
        flatten("", &value, &mut out);
        assert_eq!(out.get("auth.failed").unwrap(), "用户名或密码不正确");
        assert_eq!(out.get("plain").unwrap(), "ok");
    }

    #[test]
    fn trans_error_prefers_field_key_then_validation_rule() {
        let mut messages = HashMap::new();
        messages.insert("username.taken".into(), "该用户名已被占用".into());
        messages.insert("validation.required".into(), "请填写 :attribute".into());
        messages.insert("attributes.email".into(), "邮箱".into());
        with_catalog(messages, || {
            assert_eq!(trans_error("username.taken"), "该用户名已被占用");
            assert_eq!(trans_error("email.required"), "请填写 邮箱");
            assert_eq!(trans_error("title.required"), "请填写 title");
            assert_eq!(trans_error("unknown.zzz"), "unknown.zzz");
        });
    }
}
