//! `nx dev` — Vite HMR（前端）+ cargo-watch 热重载（后端）。

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::project::Project;
use crate::template::Mode;

#[derive(Debug, Clone)]
pub struct DevOpts {
    pub port: u16,
    pub vite_port: u16,
    pub https: bool,
    /// 只起前端
    pub frontend_only: bool,
    /// 只起后端（用已构建的 public/build，无 Vite）
    pub backend_only: bool,
    /// 关闭后端 cargo-watch（仍 `cargo run` 一次）
    pub no_reload: bool,
}

pub fn run(project: &Project, opts: DevOpts) -> Result<(), String> {
    if matches!(project.mode, Mode::Multi) {
        return Err("多应用 `nx dev` 尚未支持".into());
    }

    let app = &project.app_dir;
    let vite_origin = format!("http://127.0.0.1:{}", opts.vite_port);

    if !opts.backend_only {
        ensure_frontend_ready(app)?;
    }

    let mut kids: Vec<(&'static str, Child)> = Vec::new();

    if !opts.backend_only {
        println!("→ Vite  {vite_origin}  （前端 HMR）");
        let child = spawn_vite(app, opts.vite_port)?;
        kids.push(("vite", child));
        thread::sleep(Duration::from_millis(400));
    }

    if !opts.frontend_only {
        let watch = !opts.no_reload;
        if watch {
            ensure_cargo_watch()?;
        }
        println!(
            "→ Rust  http://127.0.0.1:{}  （{}，NAMIX_VITE_DEV={}）",
            opts.port,
            if watch {
                "cargo-watch 热重载"
            } else {
                "单次 cargo run"
            },
            if opts.backend_only { "0" } else { "1" }
        );
        let child = spawn_app(project, &opts, &vite_origin, watch)?;
        kids.push(("app", child));
    }

    if kids.is_empty() {
        return Err("--frontend-only 与 --backend-only 不能同时用".into());
    }

    println!();
    println!("✓ nx dev 已启动 — Ctrl+C 结束全部");
    if !opts.backend_only && !opts.frontend_only {
        println!("  页面:     http://127.0.0.1:{}", opts.port);
        println!("  前端 HMR: {vite_origin}  （改 TSX/CSS 即时刷新）");
        if !opts.no_reload {
            println!("  后端重载: 改 app/src 或 crates/namix* → 自动重编重启");
        }
    }
    println!();

    let code = wait_any_and_cleanup(&mut kids);
    if code == 0 {
        Ok(())
    } else {
        Err(format!("dev 进程退出 code={code}"))
    }
}

fn ensure_frontend_ready(app: &Path) -> Result<(), String> {
    let pkg = app.join("package.json");
    if !pkg.is_file() {
        return Err(format!("缺少 {}（views 前端）", pkg.display()));
    }
    if !app.join("node_modules").is_dir() {
        println!("→ npm install（app/）");
        let st = Command::new("npm")
            .args(["install"])
            .current_dir(app)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("npm install: {e}"))?;
        if !st.success() {
            return Err("npm install 失败".into());
        }
    }

    let wasm = app.join("src/views/generated/seal/namix_seal_bg.wasm");
    if !wasm.is_file() {
        println!("→ npm run build:wasm（首次生成 seal）");
        let st = Command::new("npm")
            .args(["run", "build:wasm"])
            .current_dir(app)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("build:wasm: {e}"))?;
        if !st.success() {
            return Err("build:wasm 失败".into());
        }
    }
    Ok(())
}

fn ensure_cargo_watch() -> Result<(), String> {
    if command_exists("cargo-watch") {
        return Ok(());
    }
    println!("→ 未找到 cargo-watch，正在安装（一次性）…");
    let st = Command::new("cargo")
        .args(["install", "cargo-watch", "--locked"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("cargo install cargo-watch: {e}"))?;
    if !st.success() {
        return Err(
            "安装 cargo-watch 失败。可手动: cargo install cargo-watch\n或 nx dev --no-reload"
                .into(),
        );
    }
    if !command_exists("cargo-watch") {
        return Err("cargo-watch 已安装但 PATH 中找不到（请开新终端，或加 ~/.cargo/bin）".into());
    }
    Ok(())
}

fn command_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn spawn_vite(app: &Path, vite_port: u16) -> Result<Child, String> {
    let origin = format!("http://127.0.0.1:{vite_port}");
    Command::new("npm")
        .args([
            "run",
            "dev",
            "--",
            "--host",
            "127.0.0.1",
            "--port",
            &vite_port.to_string(),
            "--strictPort",
        ])
        .current_dir(app)
        .env("NAMIX_VITE_ORIGIN", &origin)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("启动 Vite 失败: {e}"))
}

fn spawn_app(
    project: &Project,
    opts: &DevOpts,
    vite_origin: &str,
    watch: bool,
) -> Result<Child, String> {
    let port = opts.port.to_string();
    let mut run_args: Vec<String> = vec!["run".into()];
    if project.uses_workspace_package() {
        run_args.push("-p".into());
        run_args.push("app".into());
    }
    run_args.extend([
        "--bin".into(),
        "app".into(),
        "--".into(),
        "-p".into(),
        port.clone(),
    ]);
    if opts.https {
        run_args.push("--https".into());
    }

    let mut cmd = if watch {
        let mut c = Command::new("cargo-watch");
        c.current_dir(&project.app_dir);
        // 业务 + 框架 crates；忽略产物/存储
        // 只盯业务/框架源码；避开 views、generated、Boot 写出的 routes.ts
        c.args([
            "-q",
            "-d",
            "1",
            "-w",
            "src/main.rs",
            "-w",
            "src/lib.rs",
            "-w",
            "src/route.rs",
            "-w",
            "src/controllers",
            "-w",
            "src/services",
            "-w",
            "src/models",
            "-w",
            "src/listeners",
            "-w",
            "src/middleware",
            "-w",
            "src/validators",
            "-w",
            "src/events",
            "-w",
            "src/routes",
            "-w",
            "src/seeders",
            "-w",
            "src/bin",
            "-w",
            "namix.toml",
            "-w",
            "Cargo.toml",
            "-w",
            "build.rs",
            "-w",
            "../crates/namix",
            "-w",
            "../crates/namix-http",
            "-w",
            "../crates/namix-macros",
            "-w",
            "../crates/namix-build",
            "-i",
            "target",
            "-x",
        ]);
        // cargo-watch -x 吃一整条 cargo 子命令
        c.arg(run_args.join(" "));
        c
    } else {
        let mut c = Command::new("cargo");
        c.current_dir(&project.app_dir);
        c.args(&run_args);
        c
    };

    if !opts.backend_only {
        cmd.env("NAMIX_VITE_DEV", "1");
        cmd.env("NAMIX_VITE_ORIGIN", vite_origin);
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("启动 app 失败: {e}"))
}

fn wait_any_and_cleanup(kids: &mut [(&'static str, Child)]) -> i32 {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    let _ = ctrlc::set_handler(move || {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });

    loop {
        if stop.load(std::sync::atomic::Ordering::SeqCst) {
            eprintln!("· Ctrl+C — 停止全部…");
            for (n, child) in kids.iter_mut() {
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("· 已停止 {n}");
            }
            return 0;
        }

        for i in 0..kids.len() {
            match kids[i].1.try_wait() {
                Ok(Some(status)) => {
                    let name = kids[i].0;
                    let code = status.code().unwrap_or(1);
                    eprintln!("· {name} 已退出 ({code})，正在停止其余进程…");
                    for (j, (n, child)) in kids.iter_mut().enumerate() {
                        if j != i {
                            let _ = child.kill();
                            let _ = child.wait();
                            eprintln!("· 已停止 {n}");
                        }
                    }
                    return code;
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("· wait 错误: {e}");
                    for (_, child) in kids.iter_mut() {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                    return 1;
                }
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
}
