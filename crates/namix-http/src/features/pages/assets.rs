//! Vite 资源标签 + `/build/*` 静态文件。

use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use http::StatusCode;

use crate::core::content_type::ContentType;
use crate::core::controller::with_status;
use crate::core::request::Request;
use crate::core::response::Response;
use crate::core::routing::{Route, Router};

/// 开发：浏览器直接连 Vite；生产：读 `public/build/.vite/manifest.json`。
pub fn script_tags() -> String {
    let mut out = css_tags();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&js_tags());
    out
}

/// 仅样式（纯静态 SSR：不加载 React）。
pub fn css_tags() -> String {
    if vite_dev() {
        // 开发态 CSS 随 Vite 入口注入；纯静态页请先 `npm run build`
        return String::new();
    }
    match read_manifest_entry() {
        Some((css, _)) => css
            .into_iter()
            .map(|c| format!(r#"<link rel="stylesheet" href="/build/{c}"/>"#))
            .collect::<Vec<_>>()
            .join("\n"),
        None => String::new(),
    }
}

fn js_tags() -> String {
    if vite_dev() {
        let origin = vite_origin();
        // @vitejs/plugin-react 要求在入口前注入 Refresh preamble，否则整页白屏
        return format!(
            r#"<script type="module">
import RefreshRuntime from "{origin}/@react-refresh"
RefreshRuntime.injectIntoGlobalHook(window)
window.$RefreshReg$ = () => {{}}
window.$RefreshSig$ = () => (type) => type
window.__vite_plugin_react_preamble_installed__ = true
</script>
<script type="module" src="{origin}/@vite/client"></script>
<script type="module" src="{origin}/src/views/_entry.tsx"></script>"#
        );
    }

    match read_manifest_entry() {
        Some((_, js)) => format!(r#"<script type="module" src="/build/{js}"></script>"#),
        None => {
            r#"<!-- namix view: cd app && npm run build，或 export NAMIX_VITE_DEV=1 && npm run dev -->"#
                .into()
        }
    }
}

fn vite_dev() -> bool {
    match std::env::var("NAMIX_VITE_DEV") {
        Ok(v) => matches!(v.as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => {
            !Path::new("public/build/.vite/manifest.json").is_file()
                && !Path::new("public/build/manifest.json").is_file()
        }
    }
}

fn vite_origin() -> String {
    std::env::var("NAMIX_VITE_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:5173".into())
}

fn read_manifest_entry() -> Option<(Vec<String>, String)> {
    let candidates = [
        PathBuf::from("public/build/.vite/manifest.json"),
        PathBuf::from("public/build/manifest.json"),
    ];
    let raw = candidates.iter().find_map(|p| fs::read_to_string(p).ok())?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;

    let entry = v.get("src/views/_entry.tsx").or_else(|| {
        v.as_object()?
            .values()
            .find(|e| e.get("isEntry") == Some(&serde_json::Value::Bool(true)))
    })?;

    let file = strip_build_prefix(entry.get("file")?.as_str()?);
    let css = entry
        .get("css")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(strip_build_prefix))
                .collect()
        })
        .unwrap_or_default();
    Some((css, file))
}

/// Vite `base: '/build/'` 时 manifest 可能带绝对前缀；标签再拼 `/build/` 会双写。
fn strip_build_prefix(path: &str) -> String {
    let p = path.trim_start_matches('/');
    p.strip_prefix("build/").unwrap_or(p).to_string()
}

pub fn asset_routes() -> Router {
    Router::new().merge(
        Route::get("/build/*path", serve_build)
            .name("__namix.build")
            .register(),
    )
}

async fn serve_build(req: Request) -> Response {
    let path = req.param("path").unwrap_or("").to_string();
    if path.is_empty() || path.contains("..") {
        return with_status(StatusCode::NOT_FOUND, "not found");
    }
    let file = PathBuf::from("public/build").join(&path);
    match fs::read(&file) {
        Ok(bytes) => Response::new(
            StatusCode::OK,
            ContentType::from_path(&file),
            Bytes::from(bytes),
        )
        .with_header("cache-control", "public, max-age=31536000, immutable"),
        Err(_) => with_status(StatusCode::NOT_FOUND, "not found"),
    }
}
