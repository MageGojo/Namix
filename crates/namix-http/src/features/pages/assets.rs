//! Vite 资源标签 + `/build/*`（可选挂载前缀）静态文件。
//!
//! 默认公共 URL 为 `/build/…`。应用挂在子路径后（如外网 `/lr`、path-proxy 只转发
//! `/lr*`）时设置：
//! - `NAMIX_ASSET_PREFIX=/lr` → 标签与别名路由变为 `/lr/build/…`
//! - 或 `NAMIX_ASSET_BASE=/lr/build` 直接指定完整 base（无尾斜杠）
//!
//! 磁盘目录仍是 `public/build/`；始终保留根路径 `/build/*`，直连端口不受影响。

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
    let base = asset_url_base();
    match read_manifest_entry() {
        Some((css, _)) => css
            .into_iter()
            .map(|c| format!(r#"<link rel="stylesheet" href="{base}/{c}"/>"#))
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

    let base = asset_url_base();
    match read_manifest_entry() {
        Some((_, js)) => format!(r#"<script type="module" src="{base}/{js}"></script>"#),
        None => {
            r#"<!-- namix view: cd app && npm run build，或 export NAMIX_VITE_DEV=1 && npm run dev -->"#
                .into()
        }
    }
}

/// 浏览器可见的资源 URL 前缀（无尾斜杠），默认 `/build`。
///
/// `NAMIX_ASSET_BASE` 优先；否则 `NAMIX_ASSET_PREFIX=/lr` → `/lr/build`。
pub fn asset_url_base() -> String {
    if let Ok(base) = std::env::var("NAMIX_ASSET_BASE") {
        let base = base.trim();
        if !base.is_empty() {
            return normalize_url_base(base);
        }
    }
    if let Ok(prefix) = std::env::var("NAMIX_ASSET_PREFIX") {
        let prefix = normalize_mount_prefix(prefix.trim());
        if prefix.is_empty() {
            return "/build".into();
        }
        return format!("{prefix}/build");
    }
    "/build".into()
}

fn normalize_mount_prefix(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() || t == "/" {
        return String::new();
    }
    if t.starts_with('/') {
        t.to_string()
    } else {
        format!("/{t}")
    }
}

fn normalize_url_base(raw: &str) -> String {
    let t = raw.trim().trim_end_matches('/');
    if t.is_empty() || t == "/" {
        return "/build".into();
    }
    if t.starts_with('/') {
        t.to_string()
    } else {
        format!("/{t}")
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

/// Vite `base` 为 `/build/` 或 `/lr/build/` 时，manifest 可能带绝对前缀；标签再拼 base 会双写。
fn strip_build_prefix(path: &str) -> String {
    let p = path.trim_start_matches('/');
    if let Some(rest) = p.strip_prefix("build/") {
        return rest.to_string();
    }
    if let Some(idx) = p.find("/build/") {
        return p[idx + "/build/".len()..].to_string();
    }
    p.to_string()
}

pub fn asset_routes() -> Router {
    let mut router = Router::new().merge(
        Route::get("/build/*path", serve_build)
            .name("__namix.build")
            .register(),
    );
    // 子路径挂载：额外注册 `/lr/build/*`，与 HTML 标签一致；根 `/build/*` 仍保留。
    let base = asset_url_base();
    if base != "/build" {
        let pattern = format!("{base}/*path");
        router = router.merge(
            Route::get(&pattern, serve_build)
                .name("__namix.build.public")
                .register(),
        );
    }
    router
}

async fn serve_build(req: Request) -> Response {
    let path = req.param("path").unwrap_or("").to_string();
    let Some(file) = resolve_build_asset(
        Path::new("public/build"),
        Path::new("../data/public/build"),
        &path,
    ) else {
        return with_status(StatusCode::NOT_FOUND, "not found");
    };
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

fn resolve_build_asset(release_build: &Path, shared_build: &Path, raw: &str) -> Option<PathBuf> {
    let relative = safe_asset_path(raw)?;
    [release_build, shared_build]
        .into_iter()
        .map(|root| root.join(&relative))
        .find(|candidate| {
            fs::symlink_metadata(candidate)
                .map(|metadata| metadata.file_type().is_file())
                .unwrap_or(false)
        })
}

fn safe_asset_path(raw: &str) -> Option<PathBuf> {
    let path = Path::new(raw);
    if raw.is_empty()
        || raw.contains("..")
        || raw.contains('\\')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        normalize_mount_prefix, normalize_url_base, resolve_build_asset, strip_build_prefix,
    };

    fn temp_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("namix-http-assets-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn resolver_prefers_release_then_falls_back_to_shared_assets() {
        let root = temp_root();
        let release = root.join("release/public/build");
        let shared = root.join("data/public/build");
        fs::create_dir_all(release.join("assets")).unwrap();
        fs::create_dir_all(shared.join("assets")).unwrap();
        fs::write(release.join("assets/current.js"), b"current").unwrap();
        fs::write(release.join("assets/same.js"), b"release").unwrap();
        fs::write(shared.join("assets/old.js"), b"old").unwrap();
        fs::write(shared.join("assets/same.js"), b"shared").unwrap();

        assert_eq!(
            resolve_build_asset(&release, &shared, "assets/current.js"),
            Some(release.join("assets/current.js"))
        );
        assert_eq!(
            resolve_build_asset(&release, &shared, "assets/old.js"),
            Some(shared.join("assets/old.js"))
        );
        assert_eq!(
            resolve_build_asset(&release, &shared, "assets/same.js"),
            Some(release.join("assets/same.js"))
        );
        assert!(resolve_build_asset(&release, &shared, "../secret").is_none());
        assert!(resolve_build_asset(&release, &shared, "/etc/passwd").is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mount_prefix_and_url_base_normalize() {
        assert_eq!(normalize_mount_prefix("/lr"), "/lr");
        assert_eq!(normalize_mount_prefix("lr/"), "/lr");
        assert_eq!(normalize_mount_prefix("/"), "");
        assert_eq!(normalize_url_base("/lr/build/"), "/lr/build");
        assert_eq!(normalize_url_base("build"), "/build");
        assert_eq!(normalize_url_base(""), "/build");
    }

    #[test]
    fn strip_handles_default_and_prefixed_vite_base() {
        assert_eq!(strip_build_prefix("assets/a.js"), "assets/a.js");
        assert_eq!(strip_build_prefix("/build/assets/a.js"), "assets/a.js");
        assert_eq!(strip_build_prefix("build/assets/a.js"), "assets/a.js");
        assert_eq!(
            strip_build_prefix("/lr/build/assets/a.js"),
            "assets/a.js"
        );
        assert_eq!(strip_build_prefix("lr/build/assets/a.js"), "assets/a.js");
    }
}
