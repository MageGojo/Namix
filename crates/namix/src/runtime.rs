//! 生产 / 开发工作目录解析。
//!
//! - `NAMIX_HOME` 优先
//! - 否则若可执行文件旁有 `namix.toml` / `MANIFEST.json`（`dist/<ver>/`），以该目录为根
//! - 开发：`cargo run` → 回退到编译期 `app/`（由调用方传入）

use std::path::{Path, PathBuf};

/// 解析运行根目录，并 `chdir` 到该处。`dev_fallback` 一般为 `env!("CARGO_MANIFEST_DIR")`。
pub fn init_workdir(dev_fallback: impl AsRef<Path>) {
    let home = resolve_home(dev_fallback.as_ref());
    if let Err(err) = std::env::set_current_dir(&home) {
        eprintln!("namix: chdir {} failed: {err}", home.display());
    } else if std::env::var_os("NAMIX_HOME").is_some() || home.join("MANIFEST.json").is_file() {
        eprintln!("namix home → {}", home.display());
    }
}

pub fn resolve_home(dev_fallback: &Path) -> PathBuf {
    if let Ok(h) = std::env::var("NAMIX_HOME") {
        let p = PathBuf::from(h);
        if p.is_dir() {
            return p;
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
        && (dir.join("MANIFEST.json").is_file() || dir.join("namix.toml").is_file())
    {
        return dir.to_path_buf();
    }
    dev_fallback.to_path_buf()
}
