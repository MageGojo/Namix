//! `nx migrate …` — 封装 `cargo run --bin toasty -- migration …`
//! `nx seed` — 跑种子。

use std::process::{Command, Stdio};

use crate::project::Project;

pub fn generate(project: &Project) -> Result<(), String> {
    run_toasty(project, &["migration", "generate"])
}

pub fn apply(project: &Project) -> Result<(), String> {
    run_toasty(project, &["migration", "apply"])
}

pub fn status(project: &Project) -> Result<(), String> {
    // toasty 无独立 status 时用 snapshot
    run_toasty(project, &["migration", "snapshot"])
}

pub fn reset(project: &Project) -> Result<(), String> {
    run_toasty(project, &["migration", "reset"])
}

pub fn seed(project: &Project) -> Result<(), String> {
    run_bin(project, "seed", &[])
}

fn run_toasty(project: &Project, args: &[&str]) -> Result<(), String> {
    println!(
        "→ cargo run {} --bin toasty -- {}",
        pkg_flag(project),
        args.join(" ")
    );
    run_bin(project, "toasty", args)
}

fn pkg_flag(project: &Project) -> &'static str {
    if project.uses_workspace_package() {
        "-p app"
    } else {
        ""
    }
}

fn run_bin(project: &Project, bin: &str, args: &[&str]) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run");
    if project.uses_workspace_package() {
        cmd.args(["-p", "app"]);
    }
    cmd.args(["--bin", bin, "--"]);
    cmd.args(args);
    cmd.current_dir(project.cargo_cwd());
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    let status = cmd.status().map_err(|e| format!("无法执行 cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "`cargo run --bin {bin}` 失败 (exit {})",
            status.code().unwrap_or(-1)
        ))
    }
}
