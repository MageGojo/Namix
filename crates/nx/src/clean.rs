//! `nx clean` — 删除 target / node_modules 等可再生构建产物。

use std::fs;
use std::path::{Path, PathBuf};

use crate::project::Project;

/// 常见拼写也能进到这里（见 `Commands::Clean` 的 aliases）。
pub fn run(project: &Project, dry_run: bool) -> Result<(), String> {
    let (paths, skipped_env) = collect(project);
    if let Some(outside) = skipped_env {
        eprintln!("跳过 CARGO_TARGET_DIR={}（不在项目内）", outside.display());
    }
    if paths.is_empty() {
        println!("已经干净。");
        return Ok(());
    }

    let verb = if dry_run { "将删除" } else { "删除" };
    let mut bytes = 0u64;
    for path in &paths {
        let size = dir_size(path);
        bytes = bytes.saturating_add(size);
        println!(
            "  {verb} {}  ({})",
            display_path(&project.root, path),
            format_bytes(size)
        );
    }

    if dry_run {
        println!(
            "共 {} 项 · {} · 去掉 -n 才会真删",
            paths.len(),
            format_bytes(bytes)
        );
        return Ok(());
    }

    let mut failed = Vec::new();
    for path in &paths {
        if let Err(error) = remove_path(path) {
            failed.push(error);
        }
    }
    if !failed.is_empty() {
        return Err(format!(
            "部分路径没删掉（可能被 rust-analyzer / 正在跑的 cargo 占用）:\n  {}",
            failed.join("\n  ")
        ));
    }
    println!("✓ 已删除 {} 项 · {}", paths.len(), format_bytes(bytes));
    Ok(())
}

fn collect(project: &Project) -> (Vec<PathBuf>, Option<PathBuf>) {
    let mut paths = Vec::new();
    let mut push = |path: PathBuf| {
        if !path.exists() {
            return;
        }
        if !is_inside(&project.root, &path) {
            return;
        }
        if paths.iter().any(|existing| existing == &path) {
            return;
        }
        paths.push(path);
    };

    push(project.root.join("target"));
    push(project.app_dir.join("target"));
    push(project.root.join("node_modules"));
    push(project.app_dir.join("node_modules"));
    push(project.app_dir.join("public/build"));
    push(project.app_dir.join("dist"));

    let mut skipped_env = None;
    if let Some(dir) = cargo_target_dir(&project.root) {
        if is_inside(&project.root, &dir) {
            push(dir);
        } else {
            skipped_env = Some(dir);
        }
    }

    (paths, skipped_env)
}

fn cargo_target_dir(root: &Path) -> Option<PathBuf> {
    let raw = std::env::var("CARGO_TARGET_DIR").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn is_inside(root: &Path, path: &Path) -> bool {
    if path == root {
        return false;
    }
    path.starts_with(root)
}

fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| {
            let text = relative.display().to_string();
            if text.is_empty() {
                path.display().to_string()
            } else {
                text
            }
        })
        .unwrap_or_else(|_| path.display().to_string())
}

fn remove_path(path: &Path) -> Result<(), String> {
    let meta =
        fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let result = if meta.file_type().is_symlink() {
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))
    } else if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| format!("{}: {error}", path.display()))
}

fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.file_type().is_symlink() || meta.is_file() {
        return meta.len();
    }
    let mut total = 0u64;
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    for entry in entries.flatten() {
        total = total.saturating_add(dir_size(&entry.path()));
    }
    total
}

fn format_bytes(bytes: u64) -> String {
    const UNIT: f64 = 1024.0;
    let value = bytes as f64;
    if value < UNIT {
        format!("{bytes} B")
    } else if value < UNIT * UNIT {
        format!("{:.1} KB", value / UNIT)
    } else if value < UNIT * UNIT * UNIT {
        format!("{:.1} MB", value / (UNIT * UNIT))
    } else {
        format!("{:.1} GB", value / (UNIT * UNIT * UNIT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::Mode;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> (PathBuf, Project) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("namix-clean-{}-{nonce}", std::process::id()));
        let app = root.join("app");
        fs::create_dir_all(app.join("src")).expect("app src");
        fs::create_dir_all(root.join("target/debug")).expect("target");
        fs::create_dir_all(app.join("node_modules/left-pad")).expect("node_modules");
        fs::create_dir_all(app.join("public/build/assets")).expect("public/build");
        fs::create_dir_all(app.join("src/controllers")).expect("controllers");
        fs::write(root.join("target/debug/app"), b"fake-bin").expect("bin");
        fs::write(
            app.join("node_modules/left-pad/index.js"),
            b"module.exports=1",
        )
        .expect("js");
        fs::write(app.join("public/build/assets/entry.js"), b"js").expect("asset");
        fs::write(app.join("src/controllers/home.rs"), b"// keep").expect("source");
        let project = Project {
            root: root.clone(),
            app_dir: app,
            mode: Mode::Single,
        };
        (root, project)
    }

    #[test]
    fn dry_run_does_not_delete() {
        let (root, project) = fixture();
        run(&project, true).expect("dry run");
        assert!(root.join("target").is_dir());
        assert!(project.app_dir.join("node_modules").is_dir());
        assert!(project.app_dir.join("public/build").is_dir());
        assert!(project.app_dir.join("src/controllers/home.rs").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn removes_regenerable_dirs_and_keeps_source() {
        let (root, project) = fixture();
        run(&project, false).expect("clean");
        assert!(!root.join("target").exists());
        assert!(!project.app_dir.join("node_modules").exists());
        assert!(!project.app_dir.join("public/build").exists());
        assert!(project.app_dir.join("src/controllers/home.rs").is_file());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn already_clean_is_ok() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("namix-clean-empty-{}-{nonce}", std::process::id()));
        fs::create_dir_all(root.join("app/src")).expect("empty app");
        let project = Project {
            root: root.clone(),
            app_dir: root.join("app"),
            mode: Mode::Single,
        };
        run(&project, false).expect("noop");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn refuses_to_delete_project_root() {
        let root = PathBuf::from("/tmp/namix-never");
        assert!(!is_inside(&root, &root));
        assert!(is_inside(&root, &root.join("target")));
        assert!(!is_inside(&root, Path::new("/tmp/other")));
    }
}
