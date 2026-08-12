//! `nx build` — 前后端发布编译，产物落到项目根 `dist/<version>/`。

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use x25519_dalek::{PublicKey, StaticSecret};

use crate::project::Project;
use crate::template::Mode;

const DIST_DIR: &str = "dist";
const VERSION_FILE: &str = "VERSION";
const CARGO_PROFILE: &str = "release-min";
const ACTION_SEAL_KEY: &str = "storage/action_seal.key";
const ACTION_SEAL_PUBLIC_FIELD: &str = "action_seal_public_key";
const ACTION_SEAL_BUILD_MARKER: &str = ".namix-action-seal-public-key";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildStep {
    CodegenCheck,
    RouteExport,
    Frontend,
    Backend,
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

    // `--frontend-only` is a development/build-cache operation, not a release.
    // A directory under dist/<version> is always a complete, runnable artifact;
    // keeping that invariant prevents `current` from ever pointing at a bundle
    // without its server binary.
    if opts.frontend_only {
        if opts.backend_only || opts.no_frontend {
            return Err("--frontend-only 不能与 --backend-only 或 --no-frontend 同时使用".into());
        }
        println!("→ namix frontend build");
        if project.app_dir.join("Cargo.toml").is_file() {
            check_backend_codegen(project)?;
            export_routes(project)?;
        }
        let action_seal_public = resolve_action_seal_build_public(project)?;
        build_frontend(project, opts.no_obfuscate, action_seal_public)?;
        println!(
            "✓ 前端编译完成 → {}",
            project.app_dir.join("public/build").display()
        );
        println!("  未创建或切换 dist release；完整发布请运行 `nx build`");
        return Ok(());
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

    // Pin the browser/WASM build to the long-lived shared private key whenever
    // it already exists. This is resolved before npm starts so a stale
    // app/storage key cannot silently rotate every Action client away from the
    // production server key.
    let mut action_seal_public = resolve_action_seal_build_public(project)?;

    // Rust check deliberately runs first: build.rs and proc macros generate the
    // TypeScript view/action contract consumed by the frontend build. The final
    // release backend is then compiled only after that contract has passed the
    // frontend compiler.
    for step in release_build_steps(&opts) {
        match step {
            BuildStep::CodegenCheck => check_backend_codegen(project)?,
            BuildStep::RouteExport => export_routes(project)?,
            BuildStep::Frontend => {
                action_seal_public =
                    build_frontend(project, opts.no_obfuscate, action_seal_public)?;
            }
            BuildStep::Backend => build_backend(project)?,
        }
    }

    // A release may reuse an existing frontend (`--backend-only` /
    // `--no-frontend`). The marker is written only after a successful nx
    // frontend build, so stale or externally rebuilt assets cannot be paired
    // with an unrelated shared private key.
    let recorded_frontend_public = read_frontend_action_seal_marker(project)?;
    if let Some(selected) = action_seal_public
        && selected != recorded_frontend_public
    {
        return Err(format!(
            "Action seal 公钥漂移：现有前端/WASM={}，本次构建选择={}；请重新构建前端",
            encode_hex(&recorded_frontend_public),
            encode_hex(&selected)
        ));
    }
    action_seal_public = Some(recorded_frontend_public);

    let action_seal_public = ensure_shared_action_seal_key(project, action_seal_public.as_ref())?;
    stage_release(project, &staging, &version, &opts, &action_seal_public)?;
    validate_release_dir(&staging, &version)?;
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

pub(crate) fn validate_semver(v: &str) -> Result<(), String> {
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

fn release_build_steps(opts: &BuildOpts) -> Vec<BuildStep> {
    let mut steps = vec![BuildStep::CodegenCheck];
    if !opts.backend_only && !opts.no_frontend {
        steps.push(BuildStep::RouteExport);
        steps.push(BuildStep::Frontend);
    }
    steps.push(BuildStep::Backend);
    steps
}

/// Ask the compiled application to materialize its actual Router catalog
/// without opening sockets or initializing database/session drivers. This
/// keeps `routes.ts` in the same build transaction as the backend binary.
fn export_routes(project: &Project) -> Result<(), String> {
    println!("→ cargo run -p app --bin app -- --export-routes");
    let status = Command::new("cargo")
        .args([
            "run",
            "-q",
            "-p",
            "app",
            "--bin",
            "app",
            "--",
            "--export-routes",
        ])
        .current_dir(project.cargo_cwd())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| format!("执行 route export 失败: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "route export 失败 (exit {})",
            status.code().unwrap_or(-1)
        ))
    }
}

/// Run the Rust compiler once before Vite so build.rs/proc-macro generated
/// TypeScript is guaranteed to exist before frontend type-checking/bundling.
fn check_backend_codegen(project: &Project) -> Result<(), String> {
    println!("→ cargo check -p app --bin app（生成前端契约）");
    let mut cmd = Command::new("cargo");
    cmd.args(["check", "-p", "app", "--bin", "app"]);
    cmd.current_dir(project.cargo_cwd());
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());
    let status = cmd
        .status()
        .map_err(|e| format!("执行 cargo check 失败: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Rust codegen/check 失败 (exit {})",
            status.code().unwrap_or(-1)
        ))
    }
}

fn resolve_action_seal_build_public(project: &Project) -> Result<Option<[u8; 32]>, String> {
    let shared_key = project
        .root
        .join(DIST_DIR)
        .join("data")
        .join(ACTION_SEAL_KEY);
    if let Some(public) = read_optional_action_seal_public_key(&shared_key)? {
        return Ok(Some(public));
    }

    if let Ok(raw) = std::env::var("NAMIX_ACTION_SEAL_PUBLIC_KEY") {
        return parse_public_key_hex(&raw)
            .map(Some)
            .map_err(|error| format!("NAMIX_ACTION_SEAL_PUBLIC_KEY 无效: {error}"));
    }

    read_optional_action_seal_public_key(&project.app_dir.join(ACTION_SEAL_KEY))
}

fn ensure_shared_action_seal_key(
    project: &Project,
    frontend_public: Option<&[u8; 32]>,
) -> Result<[u8; 32], String> {
    let shared_key = project
        .root
        .join(DIST_DIR)
        .join("data")
        .join(ACTION_SEAL_KEY);
    let shared_public = if let Some(public) = read_optional_action_seal_public_key(&shared_key)? {
        println!("  · 保留 {}（不覆盖）", shared_key.display());
        public
    } else {
        let app_key = project.app_dir.join(ACTION_SEAL_KEY);
        let public = read_action_seal_public_key(&app_key).map_err(|error| {
            format!(
                "Action seal 私钥缺失：共享 key {} 尚未建立，应用 key {} 也不可用：{error}",
                shared_key.display(),
                app_key.display()
            )
        })?;
        if let Some(frontend_public) = frontend_public
            && frontend_public != &public
        {
            return Err(format!(
                "Action seal 公钥漂移：前端/WASM={}，待初始化的应用 key={}；请提供匹配私钥后重新构建",
                encode_hex(frontend_public),
                encode_hex(&public)
            ));
        }
        if let Some(parent) = shared_key.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建共享 Action seal 目录失败: {error}"))?;
        }
        fs::copy(&app_key, &shared_key).map_err(|error| {
            format!(
                "初始化共享 Action seal key {} 失败: {error}",
                shared_key.display()
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&shared_key, fs::Permissions::from_mode(0o600)).map_err(
                |error| {
                    format!(
                        "设置共享 Action seal key 权限失败 {}: {error}",
                        shared_key.display()
                    )
                },
            )?;
        }
        println!("  + {}（首次写入共享区）", shared_key.display());
        public
    };

    if let Some(frontend_public) = frontend_public
        && frontend_public != &shared_public
    {
        return Err(format!(
            "Action seal 公钥漂移：前端/WASM={}，共享 key={}；请用共享 key 重新构建前端",
            encode_hex(frontend_public),
            encode_hex(&shared_public)
        ));
    }

    Ok(shared_public)
}

fn read_optional_action_seal_public_key(path: &Path) -> Result<Option<[u8; 32]>, String> {
    match fs::read(path) {
        Ok(bytes) => action_seal_public_from_bytes(path, &bytes).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "读取 Action seal key {} 失败: {error}",
            path.display()
        )),
    }
}

fn read_action_seal_public_key(path: &Path) -> Result<[u8; 32], String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("读取 Action seal key {} 失败: {error}", path.display()))?;
    action_seal_public_from_bytes(path, &bytes)
}

fn action_seal_public_from_bytes(path: &Path, bytes: &[u8]) -> Result<[u8; 32], String> {
    if bytes.len() != 32 && bytes.len() != 64 {
        return Err(format!(
            "Action seal key 长度无效 {}：期望 32 或 64 bytes，实际 {}",
            path.display(),
            bytes.len()
        ));
    }

    let mut secret = [0u8; 32];
    secret.copy_from_slice(&bytes[..32]);
    let derived = *PublicKey::from(&StaticSecret::from(secret)).as_bytes();
    if bytes.len() == 64 && bytes[32..] != derived {
        return Err(format!(
            "Action seal key 的 secret/public 不一致: {}",
            path.display()
        ));
    }
    Ok(derived)
}

fn parse_public_key_hex(raw: &str) -> Result<[u8; 32], String> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("0x").unwrap_or(raw);
    let encoded = raw.as_bytes();
    if encoded.len() != 64 {
        return Err(format!(
            "公钥必须为 64 个十六进制字符，实际 {}",
            encoded.len()
        ));
    }
    let mut public = [0u8; 32];
    for (index, byte) in public.iter_mut().enumerate() {
        let offset = index * 2;
        let high = hex_nibble(encoded[offset])
            .ok_or_else(|| format!("公钥在 offset {offset} 含非十六进制字符"))?;
        let low = hex_nibble(encoded[offset + 1])
            .ok_or_else(|| format!("公钥在 offset {} 含非十六进制字符", offset + 1))?;
        *byte = (high << 4) | low;
    }
    Ok(public)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn frontend_action_seal_marker(project: &Project) -> std::path::PathBuf {
    project
        .app_dir
        .join("public/build")
        .join(ACTION_SEAL_BUILD_MARKER)
}

fn write_frontend_action_seal_marker(
    project: &Project,
    public: &[u8; 32],
) -> Result<(), String> {
    let marker = frontend_action_seal_marker(project);
    fs::write(&marker, format!("{}\n", encode_hex(public))).map_err(|error| {
        format!(
            "写入前端 Action seal 构建标记 {} 失败: {error}",
            marker.display()
        )
    })
}

fn read_frontend_action_seal_marker(project: &Project) -> Result<[u8; 32], String> {
    let marker = frontend_action_seal_marker(project);
    let raw = fs::read_to_string(&marker).map_err(|error| {
        format!(
            "前端 Action seal 构建标记缺失 {}: {error}；请先执行完整前端构建",
            marker.display()
        )
    })?;
    parse_public_key_hex(&raw).map_err(|error| {
        format!(
            "前端 Action seal 构建标记无效 {}: {error}",
            marker.display()
        )
    })
}

fn build_frontend(
    project: &Project,
    no_obfuscate: bool,
    action_seal_public: Option<[u8; 32]>,
) -> Result<Option<[u8; 32]>, String> {
    let app = &project.app_dir;
    let pkg = app.join("package.json");
    if !pkg.is_file() {
        println!("· 跳过前端：无 {}", pkg.display());
        return Ok(action_seal_public);
    }

    if !app.join("node_modules").is_dir() {
        if app.join("package-lock.json").is_file() {
            println!("→ npm ci（app/，锁定依赖）");
            run_npm(app, &["ci"])?;
        } else {
            println!("→ npm install（app/）");
            run_npm(app, &["install"])?;
        }
    }

    println!("→ npm run typecheck");
    run_npm(app, &["run", "typecheck"])?;

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
    if let Some(public) = action_seal_public {
        cmd.env("NAMIX_ACTION_SEAL_PUBLIC_KEY", encode_hex(&public));
        println!("  · Action seal WASM 使用共享公钥");
    }
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
    let public = if let Some(public) = action_seal_public {
        public
    } else {
        let app_key = project.app_dir.join("storage/action_seal.key");
        read_action_seal_public_key(&app_key).map_err(|error| {
            format!(
                "前端构建后缺少可验证的 Action seal key {}: {error}",
                app_key.display()
            )
        })?
    };
    write_frontend_action_seal_marker(project, &public)?;
    Ok(Some(public))
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
    action_seal_public: &[u8; 32],
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

    let key_dst = data_storage.join("action_seal.key");
    if key_dst.is_file() {
        println!("  · 使用 data/storage/action_seal.key（跨版本固定）");
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
    let platform = build_host_platform();
    let manifest = serde_json::json!({
        "name": "app",
        "version": version,
        "built_at": built_at,
        "profile": CARGO_PROFILE,
        "obfuscate": !opts.no_obfuscate,
        "binary_bytes": bin_size,
        "target": platform.target,
        "os": platform.os,
        "arch": platform.arch,
        "action_seal_public_key": encode_hex(action_seal_public),
    });
    fs::write(
        out.join("MANIFEST.json"),
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    )
    .map_err(|e| e.to_string())?;
    println!("  + MANIFEST.json");

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostPlatform {
    target: String,
    os: &'static str,
    arch: &'static str,
}

fn build_host_platform() -> HostPlatform {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let target = rustc_host_target().unwrap_or_else(|| fallback_host_target(os, arch));
    HostPlatform { target, os, arch }
}

fn rustc_host_target() -> Option<String> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc).arg("-vV").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::trim))
        .filter(|target| !target.is_empty())
        .map(ToOwned::to_owned)
}

fn fallback_host_target(os: &str, arch: &str) -> String {
    match os {
        "macos" => format!("{arch}-apple-darwin"),
        "linux" => format!("{arch}-unknown-linux-gnu"),
        "windows" => format!("{arch}-pc-windows-msvc"),
        other => format!("{arch}-unknown-{other}"),
    }
}

/// Validate the minimum contract of an immutable, runnable release.
///
/// This is called both immediately after staging and again by `nx start` /
/// `nx update`, so a partial upload or a manually created directory never
/// becomes `current`.
pub(crate) fn validate_release_dir(home: &Path, version: &str) -> Result<(), String> {
    validate_semver(version)?;

    if !home.is_dir() {
        return Err(format!("版本目录不存在: {}", home.display()));
    }

    let manifest_path = home.join("MANIFEST.json");
    let manifest_raw = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("发布清单缺失或不可读 {}: {error}", manifest_path.display()))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_raw)
        .map_err(|error| format!("发布清单 JSON 无效 {}: {error}", manifest_path.display()))?;
    let manifest_version = manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("发布清单缺少 version: {}", manifest_path.display()))?;
    if manifest_version != version {
        return Err(format!(
            "发布清单版本不匹配：目录={version} manifest={manifest_version}"
        ));
    }
    if manifest.get("name").and_then(serde_json::Value::as_str) != Some("app") {
        return Err(format!(
            "发布清单 name 必须为 app: {}",
            manifest_path.display()
        ));
    }
    validate_manifest_platform(
        &manifest,
        &manifest_path,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;
    validate_manifest_action_seal(&manifest, &manifest_path, home)?;

    let binary = home.join(bin_name());
    let binary_meta = fs::metadata(&binary)
        .map_err(|error| format!("发布二进制缺失 {}: {error}", binary.display()))?;
    if !binary_meta.is_file() || binary_meta.len() == 0 {
        return Err(format!("发布二进制为空或类型错误: {}", binary.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if binary_meta.permissions().mode() & 0o111 == 0 {
            return Err(format!("发布二进制没有执行权限: {}", binary.display()));
        }
    }
    let recorded_size = manifest
        .get("binary_bytes")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("发布清单缺少 binary_bytes: {}", manifest_path.display()))?;
    if recorded_size != binary_meta.len() {
        return Err(format!(
            "发布二进制大小与清单不一致：manifest={recorded_size} actual={}",
            binary_meta.len()
        ));
    }

    let public_root = home.join("public/build");
    let vite_manifest = [
        public_root.join(".vite/manifest.json"),
        public_root.join("manifest.json"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or_else(|| {
        format!(
            "发布前端清单缺失: {}",
            public_root.join(".vite/manifest.json").display()
        )
    })?;
    validate_vite_manifest(&public_root, &vite_manifest)?;
    sync_release_assets(home)?;

    Ok(())
}

fn validate_manifest_platform(
    manifest: &serde_json::Value,
    manifest_path: &Path,
    runtime_os: &str,
    runtime_arch: &str,
) -> Result<(), String> {
    let target = manifest
        .get("target")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("发布清单缺少 target: {}", manifest_path.display()))?;
    let release_os = manifest
        .get("os")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("发布清单缺少 os: {}", manifest_path.display()))?;
    let release_arch = manifest
        .get("arch")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("发布清单缺少 arch: {}", manifest_path.display()))?;

    if release_os != runtime_os || release_arch != runtime_arch {
        return Err(format!(
            "发布平台不兼容：release target={target} os={release_os} arch={release_arch}，runtime os={runtime_os} arch={runtime_arch}；请在目标宿主重新执行 nx build"
        ));
    }

    Ok(())
}

fn validate_manifest_action_seal(
    manifest: &serde_json::Value,
    manifest_path: &Path,
    home: &Path,
) -> Result<(), String> {
    let manifest_public = manifest
        .get(ACTION_SEAL_PUBLIC_FIELD)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            format!(
                "发布清单缺少 {ACTION_SEAL_PUBLIC_FIELD}: {}",
                manifest_path.display()
            )
        })?;
    let manifest_public = parse_public_key_hex(manifest_public).map_err(|error| {
        format!(
            "发布清单 {ACTION_SEAL_PUBLIC_FIELD} 无效 {}: {error}",
            manifest_path.display()
        )
    })?;

    let dist = home
        .parent()
        .ok_or_else(|| format!("release 目录缺少 dist 父目录: {}", home.display()))?;
    let shared_key = dist.join("data").join(ACTION_SEAL_KEY);
    let shared_public = read_action_seal_public_key(&shared_key).map_err(|error| {
        format!(
            "生产 Action seal key 缺失或无效，启动已终止；请配置 {}：{error}",
            shared_key.display()
        )
    })?;
    if shared_public != manifest_public {
        return Err(format!(
            "Action seal 公钥不匹配：release/WASM={}，共享 key={}（{}）；请保留共享 key 并重新构建 release",
            encode_hex(&manifest_public),
            encode_hex(&shared_public),
            shared_key.display()
        ));
    }

    Ok(())
}

/// Import Vite's content-addressed asset tree into a release-independent
/// shared directory before a candidate process starts accepting traffic.
///
/// Old and new processes can then resolve either release's hashed URLs during
/// an overlap window. Existing names are immutable: the same hash-like name
/// must always contain exactly the same bytes.
fn sync_release_assets(home: &Path) -> Result<(), String> {
    let dist = home
        .parent()
        .ok_or_else(|| format!("release 目录缺少 dist 父目录: {}", home.display()))?;
    let source = home.join("public/build/assets");
    match fs::symlink_metadata(&source) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(format!("发布 assets 必须是普通目录: {}", source.display()));
        }
        Err(error) => {
            return Err(format!(
                "发布 assets 目录缺失或不可读 {}: {error}",
                source.display()
            ));
        }
    }
    let shared = dist.join("data/public/build/assets");
    copy_assets_if_absent(&source, &shared)
}

fn copy_assets_if_absent(source: &Path, shared: &Path) -> Result<(), String> {
    match fs::symlink_metadata(shared) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(format!("共享 assets 路径必须是目录: {}", shared.display()));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(shared)
                .map_err(|error| format!("创建共享 assets {} 失败: {error}", shared.display()))?;
        }
        Err(error) => {
            return Err(format!(
                "检查共享 assets {} 失败: {error}",
                shared.display()
            ));
        }
    }

    for entry in fs::read_dir(source)
        .map_err(|error| format!("读取发布 assets {} 失败: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("读取发布 asset 条目失败: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("读取 asset 类型 {} 失败: {error}", entry.path().display()))?;
        let target = shared.join(entry.file_name());
        if file_type.is_dir() {
            copy_assets_if_absent(&entry.path(), &target)?;
        } else if file_type.is_file() {
            copy_asset_if_absent(&entry.path(), &target)?;
        } else {
            return Err(format!(
                "发布 assets 只接受普通文件和目录: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn copy_asset_if_absent(source: &Path, target: &Path) -> Result<(), String> {
    match fs::symlink_metadata(target) {
        Ok(_) => return ensure_same_asset(source, target),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!("检查共享 asset {} 失败: {error}", target.display()));
        }
    }

    let parent = target
        .parent()
        .ok_or_else(|| format!("共享 asset 缺少父目录: {}", target.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("创建共享 asset 目录 {} 失败: {error}", parent.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("asset");
    let temporary = parent.join(format!(".{file_name}.tmp-{}-{nonce}", std::process::id()));
    if let Err(error) = fs::copy(source, &temporary) {
        let _ = fs::remove_file(&temporary);
        return Err(format!(
            "暂存共享 asset {} -> {} 失败: {error}",
            source.display(),
            temporary.display()
        ));
    }

    // A hard-link publishes the complete temporary file atomically without
    // replacing an entry installed by a concurrent release process.
    match fs::hard_link(&temporary, target) {
        Ok(()) => {
            let _ = fs::remove_file(&temporary);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&temporary);
            ensure_same_asset(source, target)
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(format!("发布共享 asset {} 失败: {error}", target.display()))
        }
    }
}

fn ensure_same_asset(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(target)
        .map_err(|error| format!("读取共享 asset {} 失败: {error}", target.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("共享 asset 不是普通文件: {}", target.display()));
    }
    let source_bytes = fs::read(source)
        .map_err(|error| format!("读取发布 asset {} 失败: {error}", source.display()))?;
    let target_bytes = fs::read(target)
        .map_err(|error| format!("读取共享 asset {} 失败: {error}", target.display()))?;
    if source_bytes != target_bytes {
        return Err(format!(
            "共享 asset 名称冲突且内容不同: {}（来自 {}）",
            target.display(),
            source.display()
        ));
    }
    Ok(())
}

fn validate_vite_manifest(public_root: &Path, manifest_path: &Path) -> Result<(), String> {
    let raw = fs::read_to_string(manifest_path).map_err(|error| {
        format!(
            "读取 Vite manifest 失败 {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        format!(
            "Vite manifest JSON 无效 {}: {error}",
            manifest_path.display()
        )
    })?;
    let entries = manifest
        .as_object()
        .filter(|entries| !entries.is_empty())
        .ok_or_else(|| format!("Vite manifest 为空: {}", manifest_path.display()))?;

    let mut has_entry = false;
    for (source, entry) in entries {
        if entry.get("isEntry").and_then(serde_json::Value::as_bool) == Some(true) {
            has_entry = true;
        }
        let file = entry
            .get("file")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("Vite manifest 条目 {source:?} 缺少 file"))?;
        require_public_file(public_root, file)?;

        for field in ["css", "assets"] {
            if let Some(values) = entry.get(field) {
                let values = values
                    .as_array()
                    .ok_or_else(|| format!("Vite manifest 条目 {source:?}.{field} 必须是数组"))?;
                for value in values {
                    let path = value.as_str().ok_or_else(|| {
                        format!("Vite manifest 条目 {source:?}.{field} 含非字符串路径")
                    })?;
                    require_public_file(public_root, path)?;
                }
            }
        }

        for field in ["imports", "dynamicImports"] {
            if let Some(values) = entry.get(field) {
                let values = values
                    .as_array()
                    .ok_or_else(|| format!("Vite manifest 条目 {source:?}.{field} 必须是数组"))?;
                for value in values {
                    let key = value.as_str().ok_or_else(|| {
                        format!("Vite manifest 条目 {source:?}.{field} 含非字符串 key")
                    })?;
                    if !entries.contains_key(key) {
                        return Err(format!(
                            "Vite manifest 条目 {source:?}.{field} 引用不存在的 {key:?}"
                        ));
                    }
                }
            }
        }
    }

    if !has_entry {
        return Err(format!(
            "Vite manifest 没有 isEntry=true 条目: {}",
            manifest_path.display()
        ));
    }
    Ok(())
}

fn require_public_file(public_root: &Path, raw: &str) -> Result<(), String> {
    let raw = raw.trim_start_matches('/');
    let raw = raw.strip_prefix("build/").unwrap_or(raw);
    let relative = Path::new(raw);
    if raw.is_empty()
        || raw.contains('\\')
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(format!("Vite manifest 含非法资源路径: {raw:?}"));
    }
    let file = public_root.join(relative);
    if !file.is_file() {
        return Err(format!("Vite manifest 引用的资源缺失: {}", file.display()));
    }
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
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::project::Project;
    use crate::template::Mode;

    use super::{
        BuildOpts, BuildStep, Bump, build_host_platform, encode_hex, ensure_shared_action_seal_key,
        fallback_host_target, point_current, release_build_steps, resolve_action_seal_build_public,
        resolve_current_dir, run, sync_release_assets, validate_release_dir,
    };

    fn temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "namix-build-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn opts() -> BuildOpts {
        BuildOpts {
            version: Some("1.2.3".into()),
            bump: Bump::Patch,
            no_frontend: false,
            no_obfuscate: true,
            frontend_only: false,
            backend_only: false,
            no_db: true,
            activate: true,
        }
    }

    fn write_action_seal_key(path: &Path, secret: [u8; 32], legacy: bool) -> [u8; 32] {
        use x25519_dalek::{PublicKey, StaticSecret};

        let public = *PublicKey::from(&StaticSecret::from(secret)).as_bytes();
        let mut bytes = Vec::from(secret);
        if !legacy {
            bytes.extend_from_slice(&public);
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        public
    }

    fn write_valid_release(home: &Path, version: &str) {
        let public = write_action_seal_key(
            &home.parent().unwrap().join("data/storage/action_seal.key"),
            [7; 32],
            false,
        );
        fs::create_dir_all(home.join("public/build/.vite")).unwrap();
        fs::create_dir_all(home.join("public/build/assets")).unwrap();
        fs::write(home.join("app"), b"binary").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(home.join("app")).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(home.join("app"), permissions).unwrap();
        }
        fs::write(home.join("public/build/assets/entry.js"), b"js").unwrap();
        fs::write(home.join("public/build/assets/entry.css"), b"css").unwrap();
        fs::write(
            home.join("public/build/.vite/manifest.json"),
            serde_json::json!({
                "src/main.tsx": {
                    "file": "assets/entry.js",
                    "isEntry": true,
                    "css": ["assets/entry.css"]
                }
            })
            .to_string(),
        )
        .unwrap();
        fs::write(
            home.join("MANIFEST.json"),
            serde_json::json!({
                "name": "app",
                "version": version,
                "binary_bytes": 6,
                "target": fallback_host_target(std::env::consts::OS, std::env::consts::ARCH),
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "action_seal_public_key": encode_hex(&public)
            })
            .to_string(),
        )
        .unwrap();
    }

    #[test]
    fn build_platform_records_current_host() {
        let platform = build_host_platform();
        assert!(!platform.target.trim().is_empty());
        assert_eq!(platform.os, std::env::consts::OS);
        assert_eq!(platform.arch, std::env::consts::ARCH);
    }

    #[test]
    fn first_release_bootstraps_legacy_app_action_seal_key() {
        let root = temp_root("seal-first");
        let app = root.join("app");
        fs::create_dir_all(&app).unwrap();
        let project = Project {
            root: root.clone(),
            app_dir: app.clone(),
            mode: Mode::Single,
        };
        let expected = write_action_seal_key(&app.join("storage/action_seal.key"), [11; 32], true);

        let selected = resolve_action_seal_build_public(&project).unwrap().unwrap();
        assert_eq!(selected, expected);
        let shared = ensure_shared_action_seal_key(&project, Some(&selected)).unwrap();
        assert_eq!(shared, expected);
        assert_eq!(
            fs::read(root.join("dist/data/storage/action_seal.key"))
                .unwrap()
                .len(),
            32
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn existing_shared_action_seal_key_wins_over_app_key() {
        let root = temp_root("seal-reuse");
        let app = root.join("app");
        fs::create_dir_all(&app).unwrap();
        let project = Project {
            root: root.clone(),
            app_dir: app.clone(),
            mode: Mode::Single,
        };
        let shared_path = root.join("dist/data/storage/action_seal.key");
        let expected = write_action_seal_key(&shared_path, [21; 32], false);
        write_action_seal_key(&app.join("storage/action_seal.key"), [22; 32], false);
        let before = fs::read(&shared_path).unwrap();

        let selected = resolve_action_seal_build_public(&project).unwrap().unwrap();
        assert_eq!(selected, expected);
        assert_eq!(
            ensure_shared_action_seal_key(&project, Some(&selected)).unwrap(),
            expected
        );
        assert_eq!(fs::read(&shared_path).unwrap(), before);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn current_pointer_switches_between_immutable_releases() {
        let root = temp_root("current");
        fs::create_dir_all(root.join("1.0.0")).unwrap();
        fs::create_dir_all(root.join("1.0.1")).unwrap();

        point_current(&root, "1.0.0").unwrap();
        assert!(resolve_current_dir(&root).unwrap().ends_with("1.0.0"));
        point_current(&root, "1.0.1").unwrap();
        assert!(resolve_current_dir(&root).unwrap().ends_with("1.0.1"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn frontend_only_build_never_creates_or_activates_a_release() {
        let root = temp_root("frontend-only");
        let app = root.join("app");
        fs::create_dir_all(&app).unwrap();
        let project = Project {
            root: root.clone(),
            app_dir: app,
            mode: Mode::Single,
        };
        let mut options = opts();
        options.frontend_only = true;

        run(&project, options).unwrap();

        assert!(!root.join("dist").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn release_build_orders_codegen_before_frontend_and_backend() {
        assert_eq!(
            release_build_steps(&opts()),
            vec![
                BuildStep::CodegenCheck,
                BuildStep::RouteExport,
                BuildStep::Frontend,
                BuildStep::Backend
            ]
        );

        let mut backend_only = opts();
        backend_only.backend_only = true;
        assert_eq!(
            release_build_steps(&backend_only),
            vec![BuildStep::CodegenCheck, BuildStep::Backend]
        );
    }

    #[test]
    fn release_validator_checks_manifest_binary_and_vite_files() {
        let root = temp_root("artifact");
        let home = root.join("1.2.3");
        write_valid_release(&home, "1.2.3");
        validate_release_dir(&home, "1.2.3").unwrap();
        assert_eq!(
            fs::read(root.join("data/public/build/assets/entry.js")).unwrap(),
            b"js"
        );

        fs::remove_file(home.join("public/build/assets/entry.js")).unwrap();
        let error = validate_release_dir(&home, "1.2.3").unwrap_err();
        assert!(error.contains("资源缺失"), "{error}");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn release_assets_from_multiple_versions_accumulate_in_shared_store() {
        let root = temp_root("shared-assets");
        let first = root.join("dist/1.0.0/public/build/assets/chunks");
        let second = root.join("dist/1.0.1/public/build/assets/chunks");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("entry-v1.js"), b"v1").unwrap();
        fs::write(second.join("entry-v2.js"), b"v2").unwrap();

        sync_release_assets(&root.join("dist/1.0.0")).unwrap();
        sync_release_assets(&root.join("dist/1.0.1")).unwrap();

        let shared = root.join("dist/data/public/build/assets/chunks");
        assert_eq!(fs::read(shared.join("entry-v1.js")).unwrap(), b"v1");
        assert_eq!(fs::read(shared.join("entry-v2.js")).unwrap(), b"v2");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn shared_asset_name_with_different_content_is_rejected() {
        let root = temp_root("shared-assets-conflict");
        let first = root.join("dist/1.0.0/public/build/assets");
        let second = root.join("dist/1.0.1/public/build/assets");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("entry-same-hash.js"), b"first").unwrap();
        fs::write(second.join("entry-same-hash.js"), b"second").unwrap();

        sync_release_assets(&root.join("dist/1.0.0")).unwrap();
        let error = sync_release_assets(&root.join("dist/1.0.1")).unwrap_err();
        assert!(error.contains("内容不同"), "{error}");
        assert_eq!(
            fs::read(root.join("dist/data/public/build/assets/entry-same-hash.js")).unwrap(),
            b"first"
        );

        let _ = fs::remove_dir_all(root);
    }
}
