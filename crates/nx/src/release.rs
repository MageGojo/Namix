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

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::build::{self, BuildOpts, Bump};
use crate::project::Project;

const DIST_DIR: &str = "dist";
const PID_FILE: &str = "app.pid";
const BIN: &str = if cfg!(windows) { "app.exe" } else { "app" };

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
    let home = build::resolve_current_dir(&dist)?;
    let bin = home.join(BIN);
    if !bin.is_file() {
        return Err(format!("找不到二进制 {}", bin.display()));
    }

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
    let pid = spawn_detached(project, &home, &opts, &pid_path, &ready)?;
    wait_ready(pid, &ready, Duration::from_secs(20))?;
    println!("✓ 已启动 pid={pid}  http://127.0.0.1:{}", opts.port);
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
        println!("→ rolling update (old pid={old_pid})");
        let new_pid = spawn_detached(project, &home, &start_opts, &pid_path, &ready)?;
        if let Err(error) = wait_ready(new_pid, &ready, Duration::from_secs(25)) {
            println!("  候选进程未就绪：{error} — 停止候选并保留旧版本");
            let _ = signal_term(new_pid);
            let _ = wait_pid_exit(new_pid, Duration::from_secs(5));
            let _ = fs::write(&pid_path, format!("{old_pid}\n"));
            return Err(error);
        }

        if let Err(error) = build::point_current(&dist, &target) {
            let _ = signal_term(new_pid);
            let _ = wait_pid_exit(new_pid, Duration::from_secs(5));
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
        println!("  数据未迁移拷贝：{}", dist.join("data/storage").display());
        Ok(())
    } else {
        println!("→ 无旧进程，启动候选");
        let new_pid = spawn_detached(project, &home, &start_opts, &pid_path, &ready)?;
        if let Err(error) = wait_ready(new_pid, &ready, Duration::from_secs(25)) {
            let _ = signal_term(new_pid);
            let _ = wait_pid_exit(new_pid, Duration::from_secs(5));
            let _ = fs::remove_file(&pid_path);
            return Err(error);
        }
        if let Err(error) = build::point_current(&dist, &target) {
            let _ = signal_term(new_pid);
            let _ = wait_pid_exit(new_pid, Duration::from_secs(5));
            let _ = fs::remove_file(&pid_path);
            return Err(error);
        }
        println!("✓ 已启动 pid={new_pid}  http://127.0.0.1:{}", opts.port);
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
    if version.is_empty() || !dist.join(version).is_dir() {
        return Err(format!("版本不存在: {}", dist.join(version).display()));
    }
    Ok(version.to_string())
}

fn ensure_shared_storage(project: &Project, home: &Path) -> Result<(), String> {
    let data = project.root.join(DIST_DIR).join("data").join("storage");
    fs::create_dir_all(&data).map_err(|e| e.to_string())?;

    // 若共享区空、开发库有，首次灌入（不覆盖）
    let app_key = project.app_dir.join("storage/action_seal.key");
    let dst_key = data.join("action_seal.key");
    if app_key.is_file() && !dst_key.is_file() {
        fs::copy(&app_key, &dst_key).map_err(|e| e.to_string())?;
    }
    let app_db = project.app_dir.join("storage/namix.db");
    let dst_db = data.join("namix.db");
    if app_db.is_file() && !dst_db.is_file() {
        fs::copy(&app_db, &dst_db).map_err(|e| e.to_string())?;
    }

    // 每个不可变版本都连接到同一份数据平面。
    let link = home.join("storage");
    if !link.exists()
        && !link
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("../data/storage", &link)
                .map_err(|error| format!("创建共享 storage 链接: {error}"))?;
        }
    }
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
    install_runtime_config(&mut command, project);
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
) -> Result<u32, String> {
    let version = home
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown");
    let mut cmd = Command::new(home.join(BIN));
    cmd.current_dir(home)
        .env("NAMIX_HOME", home)
        .env("NAMIX_RELEASE_VERSION", version)
        .env("NAMIX_PIDFILE", pid_path)
        .env("NAMIX_READYFILE", ready_path)
        .args(app_args(opts))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    install_runtime_config(&mut cmd, project);

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
    Ok(pid)
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

fn install_runtime_config(command: &mut Command, project: &Project) {
    let config = project.root.join(DIST_DIR).join("data").join("namix.toml");
    if config.is_file() {
        command.env("NAMIX_CONFIG", config);
    }
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
fn wait_ready(pid: u32, ready_path: &Path, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !process_alive(pid) {
            return Err(format!("候选进程 pid={pid} 在就绪前退出"));
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
            return Err(format!("候选进程 pid={pid} 在就绪后退出"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!("候选进程 pid={pid} 就绪超时"))
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
