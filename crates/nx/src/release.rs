//! 生产运行与热更新：`nx start` / `stop` / `update` / `status`。
//!
//! 布局：
//! ```text
//! dist/
//!   data/storage/     # 跨版本共享：db、seal key（永不随版本覆盖）
//!   <ver>/app + public/build + namix.toml
//!   current -> <ver>  # 原子指针
//!   app.pid
//! ```
//!
//! 热更新：先起新进程（SO_REUSEPORT 重叠接流）→ 健康检查 → SIGTERM 旧进程排水退出。
//! 旧二进制不支持 reuseport 时自动回退为「先停后起」。

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::build::{self, BuildOpts, Bump};
use crate::project::Project;

const DIST_DIR: &str = "dist";
const PID_FILE: &str = "app.pid";
const BIN: &str = if cfg!(windows) { "app.exe" } else { "app" };

#[derive(Debug)]
struct SpawnedCandidate {
    pid: u32,
    log_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct StartOpts {
    pub port: u16,
    pub lan: bool,
    pub https: bool,
    pub https_port: Option<u16>,
    /// 前台跑（不 daemon）；默认后台
    pub foreground: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateOpts {
    pub build: bool,
    pub bump: Bump,
    pub version: Option<String>,
    pub no_frontend: bool,
    pub no_obfuscate: bool,
    pub backend_only: bool,
    pub port: u16,
    pub lan: bool,
    pub https: bool,
    pub https_port: Option<u16>,
    /// 仅切换 current，不重启进程
    pub swap_only: bool,
}

pub fn status(project: &Project) -> Result<(), String> {
    let dist = project.root.join(DIST_DIR);
    let current = build::resolve_current_dir(&dist).ok();
    let latest = fs::read_to_string(dist.join("LATEST"))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "(none)".into());
    let data_db = dist.join("data/storage/namix.db");
    let pid_path = dist.join(PID_FILE);
    let pid = read_pid(&pid_path);

    println!("dist          {}", dist.display());
    println!("LATEST        {latest}");
    if let Some(c) = &current {
        println!("current       {}", c.display());
    } else {
        println!("current       (missing — run nx build)");
    }
    println!(
        "shared db     {} ({})",
        data_db.display(),
        if data_db.is_file() {
            format!(
                "{} KiB",
                fs::metadata(&data_db).map(|m| m.len() / 1024).unwrap_or(0)
            )
        } else {
            "absent".into()
        }
    );

    match pid {
        Some(p) if process_alive(p) => {
            println!("process       pid={p}  running");
            if let Ok(ver) = running_version_hint(&dist, p) {
                println!("running ver   {ver}");
            }
        }
        Some(p) => println!("process       pid={p}  stale (not running)"),
        None => println!("process       (not started)"),
    }
    Ok(())
}

pub fn start(project: &Project, opts: StartOpts) -> Result<(), String> {
    let dist = project.root.join(DIST_DIR);
    require_production_config(project)?;
    let current = build::resolve_current_dir(&dist)?;
    let current_version = current
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("current 不是有效版本目录: {}", current.display()))?;
    let current_version = require_release(&dist, current_version)?;
    let home = dist.join(current_version);

    let pid_path = dist.join(PID_FILE);
    if let Some(pid) = read_pid(&pid_path) {
        if process_alive(pid) {
            return Err(format!(
                "已在运行 pid={pid}；先 `nx stop` 或 `nx update` 热切换"
            ));
        }
        let _ = fs::remove_file(&pid_path);
    }

    ensure_shared_storage(project, &home)?;

    println!("→ start {}", home.display());
    println!("  port {}", opts.port);

    if opts.foreground {
        return run_foreground(project, &home, &opts);
    }

    let ready = ready_file(&dist, "start");
    let candidate = spawn_detached(project, &home, &opts, &pid_path, &ready)?;
    if let Err(error) = wait_ready(
        candidate.pid,
        &ready,
        Duration::from_secs(20),
        &candidate.log_path,
    ) {
        cleanup_failed_candidate(candidate.pid, &pid_path, &ready);
        return Err(error);
    }
    println!(
        "✓ 已启动 pid={}  http://127.0.0.1:{}",
        candidate.pid, opts.port
    );
    println!("  日志 {}", candidate.log_path.display());
    println!("  数据 {}", dist.join("data/storage").display());
    Ok(())
}

pub fn stop(project: &Project) -> Result<(), String> {
    let dist = project.root.join(DIST_DIR);
    let pid_path = dist.join(PID_FILE);
    let Some(pid) = read_pid(&pid_path) else {
        println!("· 没有 pidfile，视为未运行");
        return Ok(());
    };
    if !process_alive(pid) {
        let _ = fs::remove_file(&pid_path);
        println!("· pid={pid} 已不在，清理 pidfile");
        return Ok(());
    }

    println!("→ graceful stop pid={pid}");
    signal_term(pid)?;
    if !wait_pid_exit(pid, Duration::from_secs(20)) {
        println!("  drain 超时，SIGKILL");
        signal_kill(pid)?;
        let _ = wait_pid_exit(pid, Duration::from_secs(3));
    }
    let _ = fs::remove_file(&pid_path);
    println!("✓ 已停止");
    Ok(())
}

/// 热更新：可选重新编译 → 切换 current → 新旧重叠接流 → 旧进程排水退出。
pub fn update(project: &Project, opts: UpdateOpts) -> Result<(), String> {
    let dist = project.root.join(DIST_DIR);
    require_production_config(project)?;
    let target = if opts.build {
        println!("→ build new release");
        build::run(
            project,
            BuildOpts {
                version: opts.version.clone(),
                bump: opts.bump,
                no_frontend: opts.no_frontend || opts.backend_only,
                no_obfuscate: opts.no_obfuscate,
                frontend_only: false,
                backend_only: opts.backend_only,
                // 热更新不覆盖共享数据；候选版本在就绪后才会成为 current。
                no_db: true,
                activate: false,
            },
        )?;
        latest_version(&dist)?
    } else if let Some(version) = &opts.version {
        require_release(&dist, version)?
    } else {
        latest_version(&dist)?
    };

    let home = dist.join(&target);
    ensure_shared_storage(project, &home)?;
    println!("→ candidate → {}", home.display());

    if opts.swap_only {
        build::point_current(&dist, &target)?;
        println!("✓ 已原子切换 current（运行中的旧进程保持不变）");
        return Ok(());
    }

    let pid_path = dist.join(PID_FILE);
    let old_pid = read_pid(&pid_path).filter(|&pid| process_alive(pid));
    let start_opts = StartOpts {
        port: opts.port,
        lan: opts.lan,
        https: opts.https,
        https_port: opts.https_port,
        foreground: false,
    };
    let ready = ready_file(&dist, &target);

    if let Some(old_pid) = old_pid {
        ensure_shared_session_for_rolling(project, &dist)?;
        println!("→ rolling update (old pid={old_pid})");
        let candidate = spawn_detached(project, &home, &start_opts, &pid_path, &ready)?;
        let new_pid = candidate.pid;
        if let Err(error) = wait_ready(
            new_pid,
            &ready,
            Duration::from_secs(25),
            &candidate.log_path,
        ) {
            println!("  候选进程未就绪：{error} — 停止候选并保留旧版本");
            cleanup_failed_candidate(new_pid, &pid_path, &ready);
            let _ = fs::write(&pid_path, format!("{old_pid}\n"));
            return Err(error);
        }

        if let Err(error) = build::point_current(&dist, &target) {
            cleanup_failed_candidate(new_pid, &pid_path, &ready);
            let _ = fs::write(&pid_path, format!("{old_pid}\n"));
            return Err(error);
        }

        println!("  new pid={new_pid} ready — draining old");
        let _ = signal_term(old_pid);
        if !wait_pid_exit(old_pid, Duration::from_secs(20)) {
            println!("  old pid={old_pid} drain timeout; sending SIGKILL");
            let _ = signal_kill(old_pid);
            let _ = wait_pid_exit(old_pid, Duration::from_secs(3));
        }
        let _ = fs::write(&pid_path, format!("{new_pid}\n"));
        println!("✓ 热更新完成  http://127.0.0.1:{}", opts.port);
        println!("  日志 {}", candidate.log_path.display());
        println!("  数据未迁移拷贝：{}", dist.join("data/storage").display());
        Ok(())
    } else {
        println!("→ 无旧进程，启动候选");
        let candidate = spawn_detached(project, &home, &start_opts, &pid_path, &ready)?;
        let new_pid = candidate.pid;
        if let Err(error) = wait_ready(
            new_pid,
            &ready,
            Duration::from_secs(25),
            &candidate.log_path,
        ) {
            cleanup_failed_candidate(new_pid, &pid_path, &ready);
            return Err(error);
        }
        if let Err(error) = build::point_current(&dist, &target) {
            cleanup_failed_candidate(new_pid, &pid_path, &ready);
            return Err(error);
        }
        println!("✓ 已启动 pid={new_pid}  http://127.0.0.1:{}", opts.port);
        println!("  日志 {}", candidate.log_path.display());
        Ok(())
    }
}

fn latest_version(dist: &Path) -> Result<String, String> {
    let version = fs::read_to_string(dist.join("LATEST"))
        .map_err(|_| "没有 dist/LATEST，请先 nx build 或 nx update --build".to_string())?;
    require_release(dist, version.trim())
}

fn require_release(dist: &Path, version: &str) -> Result<String, String> {
    let version = version.trim();
    build::validate_semver(version)?;
    build::validate_release_dir(&dist.join(version), version)?;
    Ok(version.to_string())
}

/// Rolling updates overlap two processes on the same port. In-process memory
/// sessions cannot be seen by the candidate, so logged-in users would bounce
/// between authenticated and anonymous responses during the drain window.
fn ensure_shared_session_for_rolling(project: &Project, dist: &Path) -> Result<(), String> {
    if allow_memory_sessions() {
        println!(
            "· NAMIX_ALLOW_MEMORY_SESSIONS set — skipping shared session preflight \
             (sessions will not survive the overlap window)"
        );
        return Ok(());
    }

    let (source, driver) = resolve_session_driver(project, dist)?;
    if session_driver_is_shared(&driver) {
        println!("· session.driver={driver} ({source}) — shared store OK for rolling update");
        return Ok(());
    }

    Err(format!(
        "rolling update blocked: session.driver={driver:?} from {source} is process-local.\n\
         Production zero-downtime requires a shared Session Store:\n\
           [session]\n\
           driver = \"file\"          # shared via dist/data/storage\n\
           path = \"./storage/sessions\"\n\
         or driver = \"redis\" with an application-wired Redis backend.\n\
         For an intentional maintenance-window cut, set NAMIX_ALLOW_MEMORY_SESSIONS=1."
    ))
}

fn allow_memory_sessions() -> bool {
    matches!(
        std::env::var("NAMIX_ALLOW_MEMORY_SESSIONS")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn session_driver_is_shared(driver: &str) -> bool {
    matches!(
        driver.trim().to_ascii_lowercase().as_str(),
        "file" | "redis" | "database" | "db"
    )
}

fn resolve_session_driver(project: &Project, dist: &Path) -> Result<(String, String), String> {
    let candidates = [
        dist.join("data/namix.toml"),
        dist.join("current").join("namix.toml"),
        project.app_dir.join("namix.toml"),
    ];
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let driver = parse_session_driver(&raw).unwrap_or_else(|| "memory".into());
        return Ok((path.display().to_string(), driver));
    }
    Ok(("(default)".into(), "memory".into()))
}

fn parse_session_driver(raw: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct File {
        #[serde(default)]
        session: Option<SessionToml>,
    }
    #[derive(serde::Deserialize, Default)]
    struct SessionToml {
        #[serde(default)]
        driver: Option<String>,
    }
    toml::from_str::<File>(raw)
        .ok()
        .and_then(|file| file.session?.driver)
        .map(|driver| driver.trim().to_ascii_lowercase())
        .filter(|driver| !driver.is_empty())
}

fn ensure_shared_storage(project: &Project, home: &Path) -> Result<(), String> {
    let data = project.root.join(DIST_DIR).join("data").join("storage");
    fs::create_dir_all(&data).map_err(|e| e.to_string())?;

    // Every runnable release must resolve `storage` to exactly the shared data
    // plane. Silently accepting a real directory would make rollback use a
    // private database/key copy and fork production state.
    let link = home.join("storage");
    match fs::symlink_metadata(&link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let actual = fs::canonicalize(&link).map_err(|error| {
                format!("release storage 链接已损坏 {}: {error}", link.display())
            })?;
            let expected = fs::canonicalize(&data)
                .map_err(|error| format!("解析共享 storage {} 失败: {error}", data.display()))?;
            if actual != expected {
                return Err(format!(
                    "release storage 指向错误: {} -> {}，期望 {}",
                    link.display(),
                    actual.display(),
                    expected.display()
                ));
            }
        }
        Ok(_) => {
            return Err(format!(
                "release storage 必须是指向 dist/data/storage 的符号链接，发现真实文件或目录: {}",
                link.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink("../data/storage", &link)
                    .map_err(|error| format!("创建共享 storage 链接: {error}"))?;
            }
            #[cfg(not(unix))]
            {
                return Err(format!(
                    "当前平台缺少 release storage 链接: {}",
                    link.display()
                ));
            }
        }
        Err(error) => {
            return Err(format!(
                "检查 release storage {} 失败: {error}",
                link.display()
            ));
        }
    }

    // The Action seal key is validated against MANIFEST.json before this
    // function runs. Production start/update never synthesizes or replaces it;
    // that would invalidate every browser bundle built for the previous key.
    // Database seeding is likewise absent here.
    Ok(())
}

fn run_foreground(project: &Project, home: &Path, opts: &StartOpts) -> Result<(), String> {
    let dist = project.root.join(DIST_DIR);
    let pid_path = dist.join(PID_FILE);
    let mut command = Command::new(home.join(BIN));
    command
        .current_dir(home)
        .env("NAMIX_HOME", home)
        .env("NAMIX_PIDFILE", &pid_path)
        .args(app_args(opts));
    install_runtime_config(&mut command, project)?;
    let status = command.status().map_err(|e| format!("启动失败: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("进程退出: {status}"))
    }
}

fn spawn_detached(
    project: &Project,
    home: &Path,
    opts: &StartOpts,
    pid_path: &Path,
    ready_path: &Path,
) -> Result<SpawnedCandidate, String> {
    let version = home
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");
    let (log_path, stdout_log, stderr_log) =
        open_candidate_log(&project.root.join(DIST_DIR), version)?;
    let mut cmd = Command::new(home.join(BIN));
    cmd.current_dir(home)
        .env("NAMIX_HOME", home)
        .env("NAMIX_RELEASE_VERSION", version)
        .env("NAMIX_PIDFILE", pid_path)
        .env("NAMIX_READYFILE", ready_path)
        .args(app_args(opts))
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(stderr_log));
    install_runtime_config(&mut cmd, project)?;

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 新会话，避免随 nx 退出
        unsafe {
            cmd.pre_exec(|| {
                libc_setsid()?;
                Ok(())
            });
        }
    }

    let child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
    let pid = child.id();
    // 立刻写一份，避免健康检查窗口丢 pid
    let _ = fs::write(pid_path, format!("{pid}\n"));
    // 不 wait；进程独立
    std::mem::forget(child);
    Ok(SpawnedCandidate { pid, log_path })
}

fn open_candidate_log(dist: &Path, version: &str) -> Result<(PathBuf, File, File), String> {
    let logs = dist.join("data/logs");
    match fs::symlink_metadata(&logs) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(format!(
                "候选进程日志路径必须是普通目录: {}",
                logs.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(&logs).map_err(|error| {
                format!("创建候选进程日志目录 {} 失败: {error}", logs.display())
            })?;
        }
        Err(error) => {
            return Err(format!(
                "检查候选进程日志目录 {} 失败: {error}",
                logs.display()
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!("设置候选进程日志目录权限 {} 失败: {error}", logs.display())
        })?;
    }

    let path = logs.join(format!("{version}.log"));
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(format!("候选进程日志路径不是普通文件: {}", path.display()));
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let stdout = options
        .open(&path)
        .map_err(|error| format!("打开候选进程日志 {} 失败: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置候选进程日志权限 {} 失败: {error}", path.display()))?;
    }
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("复制候选进程日志句柄 {} 失败: {error}", path.display()))?;
    Ok((path, stdout, stderr))
}

#[cfg(unix)]
fn libc_setsid() -> std::io::Result<()> {
    let rc = unsafe { libc_raw_setsid() };
    if rc == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
unsafe fn libc_raw_setsid() -> i32 {
    unsafe extern "C" {
        fn setsid() -> i32;
    }
    unsafe { setsid() }
}

fn require_production_config(project: &Project) -> Result<PathBuf, String> {
    let config = project.root.join(DIST_DIR).join("data").join("namix.toml");
    if !config.is_file() {
        return Err(format!(
            "生产配置缺失: {}\n请从 ops/production/namix.toml.example 创建该文件后再运行 nx start/update",
            config.display()
        ));
    }
    fs::canonicalize(&config)
        .map_err(|error| format!("解析生产配置 {} 失败: {error}", config.display()))
}

fn install_runtime_config(command: &mut Command, project: &Project) -> Result<(), String> {
    let config = require_production_config(project)?;
    command
        .env("NAMIX_CONFIG", config)
        .env("NAMIX_ENV", "production")
        .env("NAMIX_VITE_DEV", "0");
    Ok(())
}

fn app_args(opts: &StartOpts) -> Vec<String> {
    let mut args = vec!["-p".into(), opts.port.to_string()];
    if opts.lan {
        args.push("-h".into());
    }
    if opts.https {
        args.push("--https".into());
        if let Some(hp) = opts.https_port {
            args.push("--https-port".into());
            args.push(hp.to_string());
        }
    }
    args
}

fn ready_file(dist: &Path, release: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    dist.join(format!(".ready-{release}-{}-{nonce}", std::process::id()))
}

/// A ready marker is written by `Server` only after all TCP listeners bind.
/// It validates the exact candidate PID, unlike a port health probe which may
/// be answered by the old process during an SO_REUSEPORT handoff.
fn wait_ready(
    pid: u32,
    ready_path: &Path,
    timeout: Duration,
    log_path: &Path,
) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !process_alive(pid) {
            return Err(format!(
                "候选进程 pid={pid} 在就绪前退出；启动日志: {}",
                log_path.display()
            ));
        }
        if let Ok(contents) = fs::read_to_string(ready_path)
            && contents.trim() == pid.to_string()
        {
            let _ = fs::remove_file(ready_path);
            // Give async listener tasks one scheduling turn before old traffic drains.
            thread::sleep(Duration::from_millis(100));
            if process_alive(pid) {
                return Ok(());
            }
            return Err(format!(
                "候选进程 pid={pid} 在就绪后退出；启动日志: {}",
                log_path.display()
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "候选进程 pid={pid} 就绪超时；启动日志: {}",
        log_path.display()
    ))
}

fn cleanup_failed_candidate(pid: u32, pid_path: &Path, ready_path: &Path) {
    if process_alive(pid) {
        let _ = signal_term(pid);
        if !wait_pid_exit(pid, Duration::from_secs(5)) {
            let _ = signal_kill(pid);
            let _ = wait_pid_exit(pid, Duration::from_secs(2));
        }
    }
    if read_pid(pid_path) == Some(pid) {
        let _ = fs::remove_file(pid_path);
    }
    let _ = fs::remove_file(ready_path);
}

fn read_pid(path: &Path) -> Option<u32> {
    let s = fs::read_to_string(path).ok()?;
    s.trim().parse().ok()
}

fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0)
        let rc = unsafe { libc_kill(pid as i32, 0) };
        rc == 0
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

#[cfg(unix)]
unsafe fn libc_kill(pid: i32, sig: i32) -> i32 {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    unsafe { kill(pid, sig) }
}

fn signal_term(pid: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        let rc = unsafe { libc_kill(pid as i32, 15) }; // SIGTERM
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("SIGTERM pid={pid} failed"))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err("Windows 请手动停止进程".into())
    }
}

fn signal_kill(pid: u32) -> Result<(), String> {
    #[cfg(unix)]
    {
        let rc = unsafe { libc_kill(pid as i32, 9) };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("SIGKILL pid={pid} failed"))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Ok(())
    }
}

fn wait_pid_exit(pid: u32, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !process_alive(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    !process_alive(pid)
}

fn running_version_hint(dist: &Path, _pid: u32) -> Result<String, ()> {
    build::resolve_current_dir(dist)
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .ok_or(())
}

/// 供 CLI 解析 bump
pub fn parse_bump(s: &str) -> Result<Bump, String> {
    Bump::parse(s)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use crate::project::Project;
    use crate::template::Mode;

    use super::{
        cleanup_failed_candidate, ensure_shared_storage, install_runtime_config,
        open_candidate_log, parse_session_driver, require_production_config, require_release,
        session_driver_is_shared, wait_ready,
    };

    fn temp_project(label: &str) -> (PathBuf, Project) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "namix-release-{label}-{}-{nonce}",
            std::process::id()
        ));
        let app = root.join("app");
        fs::create_dir_all(&app).unwrap();
        let project = Project {
            root: root.clone(),
            app_dir: app,
            mode: Mode::Single,
        };
        (root, project)
    }

    fn write_action_seal_key(path: &Path, secret: [u8; 32], legacy: bool) -> String {
        use x25519_dalek::{PublicKey, StaticSecret};

        let public = *PublicKey::from(&StaticSecret::from(secret)).as_bytes();
        let mut bytes = Vec::from(secret);
        if !legacy {
            bytes.extend_from_slice(&public);
        }
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        public.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn write_valid_release(dist: &Path, version: &str) -> PathBuf {
        let home = dist.join(version);
        let action_seal_public =
            write_action_seal_key(&dist.join("data/storage/action_seal.key"), [7; 32], false);
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
        fs::write(
            home.join("public/build/.vite/manifest.json"),
            serde_json::json!({
                "src/main.tsx": {
                    "file": "assets/entry.js",
                    "isEntry": true
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
                "target": "fixture-host-target",
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "action_seal_public_key": action_seal_public
            })
            .to_string(),
        )
        .unwrap();
        home
    }

    #[test]
    fn parses_session_driver_from_toml() {
        assert_eq!(
            parse_session_driver(
                r#"
                [session]
                driver = "file"
                path = "./storage/sessions"
                "#
            )
            .as_deref(),
            Some("file")
        );
        assert!(parse_session_driver("[apps.main]\nport = 1").is_none());
    }

    #[test]
    fn shared_driver_matrix_matches_framework() {
        assert!(!session_driver_is_shared("memory"));
        assert!(session_driver_is_shared("file"));
        assert!(session_driver_is_shared("redis"));
    }

    #[test]
    fn release_preflight_rejects_paths_and_partial_artifacts() {
        let (root, _project) = temp_project("artifact");
        let dist = root.join("dist");
        fs::create_dir_all(dist.join("data")).unwrap();
        let home = write_valid_release(&dist, "1.2.3");

        assert_eq!(require_release(&dist, "1.2.3").unwrap(), "1.2.3");
        assert!(require_release(&dist, "data").is_err());
        assert!(require_release(&dist, "../1.2.3").is_err());

        fs::remove_file(home.join("MANIFEST.json")).unwrap();
        let error = require_release(&dist, "1.2.3").unwrap_err();
        assert!(error.contains("发布清单"), "{error}");
        write_valid_release(&dist, "1.2.3");

        fs::remove_file(home.join("app")).unwrap();
        let error = require_release(&dist, "1.2.3").unwrap_err();
        assert!(error.contains("发布二进制"), "{error}");
        fs::write(home.join("app"), b"binary").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(home.join("app")).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(home.join("app"), permissions).unwrap();
        }

        fs::remove_file(home.join("public/build/.vite/manifest.json")).unwrap();
        let error = require_release(&dist, "1.2.3").unwrap_err();
        assert!(error.contains("前端清单"), "{error}");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn release_preflight_rejects_incompatible_or_legacy_platform_manifest() {
        let (root, _project) = temp_project("platform");
        let dist = root.join("dist");
        let home = write_valid_release(&dist, "1.2.3");
        let manifest_path = home.join("MANIFEST.json");
        let mut manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();

        let incompatible_os = if std::env::consts::OS == "linux" {
            "macos"
        } else {
            "linux"
        };
        manifest["os"] = serde_json::Value::String(incompatible_os.into());
        fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
        let error = require_release(&dist, "1.2.3").unwrap_err();
        assert!(error.contains("发布平台不兼容"), "{error}");
        assert!(error.contains("runtime os="), "{error}");

        manifest["os"] = serde_json::Value::String(std::env::consts::OS.into());
        manifest["arch"] = serde_json::Value::String("incompatible-arch".into());
        fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
        let error = require_release(&dist, "1.2.3").unwrap_err();
        assert!(error.contains("发布平台不兼容"), "{error}");
        assert!(error.contains("arch=incompatible-arch"), "{error}");

        manifest["arch"] = serde_json::Value::String(std::env::consts::ARCH.into());
        manifest.as_object_mut().unwrap().remove("target");
        fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
        let error = require_release(&dist, "1.2.3").unwrap_err();
        assert!(error.contains("发布清单缺少 target"), "{error}");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn release_preflight_rejects_action_seal_key_drift_or_missing_key() {
        let (root, _project) = temp_project("seal-preflight");
        let dist = root.join("dist");
        write_valid_release(&dist, "1.2.3");
        let shared_key = dist.join("data/storage/action_seal.key");

        write_action_seal_key(&shared_key, [8; 32], true);
        let error = require_release(&dist, "1.2.3").unwrap_err();
        assert!(error.contains("Action seal 公钥不匹配"), "{error}");

        fs::remove_file(&shared_key).unwrap();
        let error = require_release(&dist, "1.2.3").unwrap_err();
        assert!(error.contains("Action seal key 缺失或无效"), "{error}");
        assert!(
            error.contains("dist/data/storage/action_seal.key"),
            "{error}"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detached_candidate_logs_diagnostics_and_cleanup_removes_markers() {
        let (root, _project) = temp_project("candidate-log");
        let dist = root.join("dist");
        let (log_path, mut stdout, mut stderr) = open_candidate_log(&dist, "1.2.3").unwrap();
        writeln!(stdout, "stdout diagnostic").unwrap();
        writeln!(stderr, "stderr diagnostic").unwrap();
        drop(stdout);
        drop(stderr);
        let contents = fs::read_to_string(&log_path).unwrap();
        assert!(contents.contains("stdout diagnostic"));
        assert!(contents.contains("stderr diagnostic"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&log_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(log_path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }

        let fake_pid = i32::MAX as u32;
        let ready = dist.join(".ready-test");
        let pid_file = dist.join("app.pid");
        fs::write(&ready, format!("{fake_pid}\n")).unwrap();
        fs::write(&pid_file, format!("{fake_pid}\n")).unwrap();
        let error = wait_ready(fake_pid, &ready, Duration::from_millis(1), &log_path).unwrap_err();
        assert!(error.contains(&log_path.display().to_string()), "{error}");
        cleanup_failed_candidate(fake_pid, &pid_file, &ready);
        assert!(!pid_file.exists());
        assert!(!ready.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn production_commands_require_and_install_stable_config() {
        let (root, project) = temp_project("config");
        let error = require_production_config(&project).unwrap_err();
        assert!(error.contains("dist/data/namix.toml"), "{error}");

        let config = root.join("dist/data/namix.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "[apps.main]\nport = 3000\n").unwrap();

        let mut command = Command::new("fixture-app");
        install_runtime_config(&mut command, &project).unwrap();
        let envs = command
            .get_envs()
            .filter_map(|(key, value)| Some((key.to_str()?, value?.to_str()?)))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(envs.get("NAMIX_ENV"), Some(&"production"));
        assert_eq!(envs.get("NAMIX_VITE_DEV"), Some(&"0"));
        assert_eq!(
            envs.get("NAMIX_CONFIG").copied(),
            fs::canonicalize(config).unwrap().to_str()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn shared_storage_creates_and_validates_exact_link_without_seeding_db() {
        let (root, project) = temp_project("storage-link");
        let home = root.join("dist/1.2.3");
        fs::create_dir_all(project.app_dir.join("storage")).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(project.app_dir.join("storage/namix.db"), b"development-db").unwrap();

        ensure_shared_storage(&project, &home).unwrap();

        let link = home.join("storage");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::canonicalize(&link).unwrap(),
            fs::canonicalize(root.join("dist/data/storage")).unwrap()
        );
        assert!(!root.join("dist/data/storage/namix.db").exists());

        // Re-validating the same link is idempotent.
        ensure_shared_storage(&project, &home).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(unix)]
    fn shared_storage_rejects_real_directory_and_wrong_symlink() {
        let (root, project) = temp_project("storage-invalid");
        let real_home = root.join("dist/1.2.3");
        fs::create_dir_all(real_home.join("storage")).unwrap();
        let error = ensure_shared_storage(&project, &real_home).unwrap_err();
        assert!(error.contains("符号链接"), "{error}");

        let wrong_home = root.join("dist/1.2.4");
        fs::create_dir_all(&wrong_home).unwrap();
        fs::create_dir_all(root.join("dist/wrong-storage")).unwrap();
        std::os::unix::fs::symlink("../wrong-storage", wrong_home.join("storage")).unwrap();
        let error = ensure_shared_storage(&project, &wrong_home).unwrap_err();
        assert!(error.contains("指向错误"), "{error}");

        let _ = fs::remove_dir_all(root);
    }
}
