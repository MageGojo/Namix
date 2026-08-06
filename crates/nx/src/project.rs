//! 定位业务包 `app/`，识别单应用 / 多应用。

use std::fs;
use std::path::{Path, PathBuf};

use crate::template::Mode;

#[derive(Clone, Debug)]
pub struct Project {
    /// 工作区根（含成员 `app` 的 Cargo.toml），或 app 自身。
    pub root: PathBuf,
    /// 业务包目录（含 namix.toml / src）。
    pub app_dir: PathBuf,
    pub mode: Mode,
}

impl Project {
    pub fn src_dir(&self) -> PathBuf {
        self.app_dir.join("src")
    }

    pub fn models_dir(&self) -> PathBuf {
        match self.mode {
            Mode::Single => self.src_dir().join("models"),
            Mode::Multi => self.src_dir().join("common/models"),
        }
    }

    pub fn services_dir(&self) -> PathBuf {
        match self.mode {
            Mode::Single => self.src_dir().join("services"),
            Mode::Multi => self.src_dir().join("common/services"),
        }
    }

    pub fn seeders_dir(&self) -> PathBuf {
        match self.mode {
            Mode::Single => self.src_dir().join("seeders"),
            Mode::Multi => self.src_dir().join("common/seeders"),
        }
    }

    pub fn namix_toml(&self) -> PathBuf {
        self.app_dir.join("namix.toml")
    }

    pub fn toasty_toml(&self) -> PathBuf {
        self.app_dir.join("Toasty.toml")
    }

    pub fn build_rs(&self) -> PathBuf {
        self.app_dir.join("build.rs")
    }

    pub fn registry_rs(&self) -> PathBuf {
        self.models_dir().join("registry.rs")
    }

    /// `cargo run` 的工作目录：优先 workspace 根。
    pub fn cargo_cwd(&self) -> &Path {
        &self.root
    }

    pub fn uses_workspace_package(&self) -> bool {
        self.root != self.app_dir
            && self.root.join("Cargo.toml").is_file()
            && fs::read_to_string(self.root.join("Cargo.toml"))
                .map(|s| s.contains("members") && s.contains("app"))
                .unwrap_or(false)
    }
}

/// 从当前目录向上查找 Namix 业务项目。
pub fn discover() -> Result<Project, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let mut dir = cwd.as_path();
    loop {
        if let Some(p) = try_project_at(dir) {
            return Ok(p);
        }
        dir = dir.parent().ok_or_else(|| {
            String::from("未找到 Namix 项目（缺少 namix.toml）。请在项目根或 app/ 下执行。")
        })?;
    }
}

fn try_project_at(dir: &Path) -> Option<Project> {
    // dir 本身就是 app/
    if dir.join("namix.toml").is_file() && dir.join("src").is_dir() {
        let mode = detect_mode(dir)?;
        let root = workspace_root_for(dir);
        return Some(Project {
            root,
            app_dir: dir.to_path_buf(),
            mode,
        });
    }
    // dir/app/
    let app = dir.join("app");
    if app.join("namix.toml").is_file() && app.join("src").is_dir() {
        let mode = detect_mode(&app)?;
        return Some(Project {
            root: dir.to_path_buf(),
            app_dir: app,
            mode,
        });
    }
    None
}

fn workspace_root_for(app_dir: &Path) -> PathBuf {
    if let Some(parent) = app_dir.parent() {
        let cargo = parent.join("Cargo.toml");
        if cargo.is_file()
            && let Ok(s) = fs::read_to_string(&cargo)
            && s.contains("[workspace]")
        {
            return parent.to_path_buf();
        }
    }
    app_dir.to_path_buf()
}

fn detect_mode(app_dir: &Path) -> Option<Mode> {
    let build = fs::read_to_string(app_dir.join("build.rs")).unwrap_or_default();
    if build.contains("sync_single") {
        return Some(Mode::Single);
    }
    if build.contains("namix_build::sync(") || build.contains("namix_build::sync()") {
        return Some(Mode::Multi);
    }
    let src = app_dir.join("src");
    if src.join("www").is_dir() || src.join("common").is_dir() {
        return Some(Mode::Multi);
    }
    if src.join("controllers").is_dir() || src.join("main.rs").is_file() {
        return Some(Mode::Single);
    }
    None
}

/// `Article` / `article` / `user_profile` → (`Article`, `article`)
pub fn normalize_type_name(raw: &str) -> Result<(String, String), String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("名称不能为空".into());
    }
    let snake = to_snake(raw);
    if snake.is_empty() || !snake.chars().next().unwrap().is_ascii_alphabetic() {
        return Err(format!("非法名称: {raw}"));
    }
    if !snake
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!("非法名称: {raw}"));
    }
    let pascal = to_pascal(&snake);
    Ok((pascal, snake))
}

fn to_snake(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c == '-' || c == ' ' {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            continue;
        }
        if c.is_uppercase() {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim_matches('_').to_string()
}

fn to_pascal(snake: &str) -> String {
    snake
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            normalize_type_name("Article").unwrap(),
            ("Article".into(), "article".into())
        );
        assert_eq!(
            normalize_type_name("user_profile").unwrap(),
            ("UserProfile".into(), "user_profile".into())
        );
    }
}
