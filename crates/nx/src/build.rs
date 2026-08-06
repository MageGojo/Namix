//! `nx build` — 前后端发布编译，产物落到项目根 `dist/<version>/`。

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::project::Project;
use crate::template::Mode;

const DIST_DIR: &str = "dist";
const VERSION_FILE: &str = "VERSION";
const CARGO_PROFILE: &str = "release-min";

#[derive(Debug, Clone)]
pub struct BuildOpts {
    /// 显式版本，如 `1.2.3`；与 bump 互斥（显式优先）
    pub version: Option<String>,
    /// 自动递增：major / minor / patch（默认 patch）
    pub bump: Bump,
    /// 跳过前端 npm build
    pub no_frontend: bool,
    /// 关闭 JS 混淆（仍 minify）
    pub no_obfuscate: bool,
    /// 只编前端
    pub frontend_only: bool,
    /// 只编后端
    pub backend_only: bool,
    /// 不打包 `storage/namix.db`（默认有则打入）
    pub no_db: bool,
    /// 构建成功后是否切换 `dist/current`。`nx update --build` 会先保留旧版本，
    /// 等候选进程就绪后再原子切换。
    pub activate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bump {
    Major,
    Minor,
    Patch,
}

impl Bump {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "major" => Ok(Self::Major),
            "minor" => Ok(Self::Minor),
            "patch" | "" => Ok(Self::Patch),
            other => Err(format!("未知 --bump={other}，可选 major|minor|patch")),
        }
    }
}

pub fn run(project: &Project, opts: BuildOpts) -> Result<(), String> {
    if matches!(project.mode, Mode::Multi) {
        return Err("多应用 `nx build` 尚未支持；请先用单应用，或手动 cargo/npm".into());
    }

    let version = resolve_version(project, &opts)?;
    let dist_root = project.root.join(DIST_DIR);
    let out = dist_root.join(&version);
    let staging = dist_root.join(format!(".staging-{version}-{}", std::process::id()));
    println!("→ namix build  version={version}");
    println!("  输出目录 {}", out.display());

    // 发布目录不可变：绝不覆盖正在运行或可回滚的版本。
    if out.exists() {
        return Err(format!(
            "版本 {version} 已存在；发布版本不可覆盖，请使用新的 --ver 或默认自动递增"
        ));
    }
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| format!("清理残留 staging: {e}"))?;
    }
    fs::create_dir_all(&staging).map_err(|e| format!("创建 staging: {e}"))?;

    if !opts.backend_only && !opts.no_frontend {
        build_frontend(project, opts.no_obfuscate)?;
    }

    if !opts.frontend_only {
        build_backend(project)?;
    }

    stage_release(project, &staging, &version, &opts)?;
    fs::rename(&staging, &out).map_err(|e| format!("发布 staging: {e}"))?;
    write_root_version(project, &version)?;
    write_text_atomically(&dist_root.join("LATEST"), &format!("{version}\n"))?;
    if opts.activate {
        point_current(&dist_root, &version)?;
    } else {
        println!("  · 已保留旧 current，待候选进程就绪后切换");
    }

    println!();
    println!("✓ 编译完成 → {}", out.display());
    println!("  版本 {version}（已写入 {}/{VERSION_FILE}）", DIST_DIR);
    println!(
        "  数据目录 {}/data/storage/（跨版本共享，热更新不丢库）",
        DIST_DIR
    );
    println!("  运行: nx start -p 3000");
    println!("  或:   cd {} && ./app -p 3000", out.display());
    Ok(())
}

/// 原子切换 `dist/current` → `<version>`。
///
/// 先创建同目录临时指针，再 `rename` 覆盖旧指针。这样反向代理、静态文件和
/// 回滚工具不会观察到「current 不存在」的中间态。
pub fn point_current(dist_root: &Path, version: &str) -> Result<(), String> {
    if !dist_root.join(version).is_dir() {
        return Err(format!(
            "版本目录不存在: {}",
            dist_root.join(version).display()
        ));
    }
    let current = dist_root.join("current");
    let temporary = dist_root.join(format!(".current-{version}-{}", std::process::id()));
    #[cfg(unix)]
    {
        let _ = fs::remove_file(&temporary);
        std::os::unix::fs::symlink(version, &temporary)
            .map_err(|e| format!("创建临时 current → {version}: {e}"))?;
        fs::rename(&temporary, &current).map_err(|e| format!("原子切换 current: {e}"))?;
        println!("  → current → {version}");
    }
    #[cfg(not(unix))]
    {
        write_text_atomically(&current, &format!("{version}\n"))?;
        println!("  → current = {version} (text pointer)");
    }
    Ok(())
}

fn write_text_atomically(path: &Path, contents: &str) -> Result<(), String> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, contents).map_err(|e| e.to_string())?;
    fs::rename(&temporary, path).map_err(|e| format!("原子写入 {}: {e}", path.display()))
}

pub fn resolve_current_dir(dist_root: &Path) -> Result<std::path::PathBuf, String> {
    let current = dist_root.join("current");
    #[cfg(unix)]
    {
        if current.is_symlink() || current.is_dir() {
            return fs::canonicalize(&current).map_err(|e| format!("解析 current: {e}"));
        }
    }
    #[cfg(not(unix))]
    {
        if current.is_file() {
            let v = fs::read_to_string(&current).map_err(|e| e.to_string())?;
            let v = v.trim();
            let dir = dist_root.join(v);
            if dir.is_dir() {
                return Ok(dir);
            }
        }
    }
    // 回退 LATEST
    let latest = dist_root.join("LATEST");
    let v = fs::read_to_string(&latest)
        .map_err(|_| "没有 dist/current 或 dist/LATEST，请先 nx build".to_string())?;
    let v = v.trim();
    let dir = dist_root.join(v);
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(format!("版本目录不存在: {}", dir.display()))
    }
}

fn resolve_version(project: &Project, opts: &BuildOpts) -> Result<String, String> {
    if let Some(v) = &opts.version {
        let v = v.trim().trim_start_matches('v');
        validate_semver(v)?;
        return Ok(v.to_string());
    }

    let path = project.root.join(DIST_DIR).join(VERSION_FILE);
    if let Ok(s) = fs::read_to_string(&path) {
        let current = s.trim().trim_start_matches('v');
        if validate_semver(current).is_ok() {
            return Ok(bump_semver(current, opts.bump));
        }
    }

    // 首次：用 workspace version，不递增
    Ok(read_cargo_version(project).unwrap_or_else(|| "0.1.0".into()))
}

fn read_cargo_version(project: &Project) -> Option<String> {
    let toml = fs::read_to_string(project.root.join("Cargo.toml")).ok()?;
    // [workspace.package] version = "x.y.z"
    let mut in_ws = false;
    for line in toml.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_ws = line == "[workspace.package]";
            continue;
        }
        if in_ws && let Some(rest) = line.strip_prefix("version") {
            let rest = rest.trim().trim_start_matches('=').trim();
            let v = rest.trim_matches('"').trim_matches('\'');
            if validate_semver(v).is_ok() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn validate_semver(v: &str) -> Result<(), String> {
    let parts: Vec<_> = v.split('.').collect();
    if parts.len() != 3 {
        return Err(format!("版本须为 x.y.z，收到 `{v}`"));
    }
    for p in parts {
        if p.parse::<u64>().is_err() {
            return Err(format!("版本须为 x.y.z（数字），收到 `{v}`"));
        }
    }
    Ok(())
}

fn bump_semver(current: &str, bump: Bump) -> String {
    let mut parts = current
        .split('.')
        .filter_map(|p| p.parse::<u64>().ok())
        .collect::<Vec<_>>();
    while parts.len() < 3 {
        parts.push(0);
    }
    match bump {
        Bump::Major => {
            parts[0] += 1;
            parts[1] = 0;
            parts[2] = 0;
        }
        Bump::Minor => {
            parts[1] += 1;
            parts[2] = 0;
        }
        Bump::Patch => {
            parts[2] += 1;
        }
    }
    format!("{}.{}.{}", parts[0], parts[1], parts[2])
}

fn write_root_version(project: &Project, version: &str) -> Result<(), String> {
    let dir = project.root.join(DIST_DIR);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(VERSION_FILE), format!("{version}\n")).map_err(|e| e.to_string())
}

fn build_frontend(project: &Project, no_obfuscate: bool) -> Result<(), String> {
    let app = &project.app_dir;
    let pkg = app.join("package.json");
    if !pkg.is_file() {
        println!("· 跳过前端：无 {}", pkg.display());
        return Ok(());
    }

    if !app.join("node_modules").is_dir() {
        println!("→ npm install（app/）");
        run_npm(app, &["install"])?;
    }

    println!(
        "→ npm run build（最小体积{}）",
        if no_obfuscate {
            "，无混淆"
        } else {
            " + JS 混淆"
        }
    );
    let mut cmd = Command::new("npm");
    cmd.args(["run", "build"]);
    cmd.current_dir(app);
    cmd.env("NAMIX_MIN_SIZE", "1");
    if no_obfuscate {
        cmd.env("NAMIX_OBFUSCATE", "0");
    } else {
        cmd.env("NAMIX_OBFUSCATE", "1");
    }
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    let status = cmd.status().map_err(|e| format!("无法执行 npm: {e}"))?;
    if !status.success() {
        return Err(format!(
            "前端构建失败 (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

fn run_npm(cwd: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new("npm");
    cmd.args(args);
    cmd.current_dir(cwd);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    let status = cmd.status().map_err(|e| format!("无法执行 npm: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("npm {} 失败", args.join(" ")))
    }
}

fn build_backend(project: &Project) -> Result<(), String> {
    println!("→ cargo build -p app --bin app --profile {CARGO_PROFILE}");
    let mut cmd = Command::new("cargo");
    cmd.args([
        "build",
        "-p",
        "app",
        "--bin",
        "app",
        "--profile",
        CARGO_PROFILE,
    ]);
    cmd.current_dir(project.cargo_cwd());
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    let status = cmd.status().map_err(|e| format!("无法执行 cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "后端构建失败 (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

fn stage_release(
    project: &Project,
    out: &Path,
    version: &str,
    opts: &BuildOpts,
) -> Result<(), String> {
    // 二进制
    if !opts.frontend_only {
        let bin_src = project
            .root
            .join("target")
            .join(CARGO_PROFILE)
            .join(bin_name());
        if !bin_src.is_file() {
            return Err(format!("找不到二进制 {}", bin_src.display()));
        }
        let bin_dst = out.join(bin_name());
        fs::copy(&bin_src, &bin_dst).map_err(|e| format!("复制二进制: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&bin_dst)
                .map_err(|e| e.to_string())?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&bin_dst, perms).map_err(|e| e.to_string())?;
        }
        println!("  + {}", bin_dst.file_name().unwrap().to_string_lossy());
    }

    // 开发配置原样进入发布包。生产启动器优先读取共享数据平面的
    // `NAMIX_CONFIG`，因此密钥、域名和 TLS 拓扑不会随版本覆盖。
    let toml_src = project.app_dir.join("namix.toml");
    if toml_src.is_file() {
        fs::copy(&toml_src, out.join("namix.toml")).map_err(|e| e.to_string())?;
        println!("  + namix.toml");
    }

    // 前端产物（backend-only 时若本地已有 build 也一并打入）
    let build_src = project.app_dir.join("public/build");
    if build_src.is_dir() {
        let build_dst = out.join("public/build");
        copy_dir(&build_src, &build_dst)?;
        println!("  + public/build/");
    } else if !opts.backend_only && !opts.no_frontend {
        return Err("缺少 app/public/build — 前端未构建成功".into());
    }

    // 共享数据平面：dist/data/storage（跨版本；热更新绝不覆盖已有 db/key）
    let data_storage = project.root.join(DIST_DIR).join("data").join("storage");
    fs::create_dir_all(&data_storage).map_err(|e| e.to_string())?;

    let key_src = project.app_dir.join("storage/action_seal.key");
    let key_dst = data_storage.join("action_seal.key");
    if key_src.is_file() && !key_dst.is_file() {
        fs::copy(&key_src, &key_dst).map_err(|e| e.to_string())?;
        println!("  + data/storage/action_seal.key（首次写入共享区）");
    } else if key_dst.is_file() {
        println!("  · 保留 data/storage/action_seal.key（不覆盖）");
    }

    let db_src = project.app_dir.join("storage/namix.db");
    let db_dst = data_storage.join("namix.db");
    if !opts.no_db && db_src.is_file() && !db_dst.is_file() {
        fs::copy(&db_src, &db_dst).map_err(|e| e.to_string())?;
        let kb = fs::metadata(&db_dst).map(|m| m.len() / 1024).unwrap_or(0);
        println!("  + data/storage/namix.db（首次写入共享区，{kb} KiB）");
    } else if db_dst.is_file() {
        println!("  · 保留 data/storage/namix.db（不覆盖，热更新保数据）");
    } else if opts.no_db {
        println!("  · 跳过种子 namix.db（--no-db；共享区已有则继续用）");
    } else {
        println!("  · 无 namix.db（首启将按 schema 建库到 data/storage）");
    }

    // 版本目录 storage → 共享区（相对符号链接）
    link_version_storage(out, &data_storage)?;

    // 清单
    let built_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let bin_size = out
        .join(bin_name())
        .metadata()
        .map(|m| m.len())
        .unwrap_or(0);
    let manifest = serde_json::json!({
        "name": "app",
        "version": version,
        "built_at": built_at,
        "profile": CARGO_PROFILE,
        "obfuscate": !opts.no_obfuscate,
        "binary_bytes": bin_size,
    });
    fs::write(
        out.join("MANIFEST.json"),
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    println!("  + MANIFEST.json");

    Ok(())
}

fn bin_name() -> &'static str {
    if cfg!(windows) { "app.exe" } else { "app" }
}

fn link_version_storage(out: &Path, data_storage: &Path) -> Result<(), String> {
    let storage_out = out.join("storage");
    if let Ok(meta) = storage_out.symlink_metadata() {
        if meta.file_type().is_symlink() || meta.is_file() {
            fs::remove_file(&storage_out).map_err(|e| e.to_string())?;
        } else if meta.is_dir() {
            fs::remove_dir_all(&storage_out).map_err(|e| e.to_string())?;
        }
    }

    #[cfg(unix)]
    {
        // dist/<ver>/storage → ../data/storage
        let _ = data_storage;
        std::os::unix::fs::symlink("../data/storage", &storage_out)
            .map_err(|e| format!("symlink storage: {e}"))?;
        println!("  + storage → ../data/storage");
    }
    #[cfg(not(unix))]
    {
        // Windows：复制共享区内容（无法可靠 symlink 时）
        copy_dir(data_storage, &storage_out)?;
        println!("  + storage/（从 data/storage 复制）");
    }
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else if ty.is_file() {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(entry.path(), &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{point_current, resolve_current_dir};

    #[test]
    #[cfg(unix)]
    fn current_pointer_switches_between_immutable_releases() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("namix-build-current-{nonce}"));
        fs::create_dir_all(root.join("1.0.0")).unwrap();
        fs::create_dir_all(root.join("1.0.1")).unwrap();

        point_current(&root, "1.0.0").unwrap();
        assert!(resolve_current_dir(&root).unwrap().ends_with("1.0.0"));
        point_current(&root, "1.0.1").unwrap();
        assert!(resolve_current_dir(&root).unwrap().ends_with("1.0.1"));

        let _ = fs::remove_dir_all(root);
    }
}
