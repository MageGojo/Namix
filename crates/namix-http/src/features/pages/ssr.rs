//! Node SSR worker：读 Vite 打出的 `public/build/ssr/_ssr.js`，按行 JSON 渲染。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    started: Instant,
}

static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

/// 用 Node 渲染组件 HTML；失败时返回 Err（上层回退 SPA）。
pub fn render_html(component: &str, props: &Value, url: &str) -> Result<String, String> {
    let payload = json!({
        "component": component,
        "props": props,
        "url": url,
    });
    let line = payload.to_string();

    let mut guard = WORKER
        .lock()
        .map_err(|_| "ssr worker mutex poisoned".to_string())?;

    if let Some(w) = guard.as_mut() {
        if w.child.try_wait().ok().flatten().is_some() {
            *guard = None;
        } else if w.started.elapsed() > Duration::from_secs(30 * 60) {
            let _ = w.child.kill();
            *guard = None;
        }
    }

    if guard.is_none() {
        *guard = Some(spawn_worker()?);
    }

    let worker = guard.as_mut().unwrap();
    match request(worker, &line) {
        Ok(html) => Ok(html),
        Err(err) => {
            let _ = worker.child.kill();
            *guard = None;
            // 冷启动再试一次
            let mut fresh = spawn_worker()?;
            let html = request(&mut fresh, &line).map_err(|e| format!("{err}; retry: {e}"))?;
            *guard = Some(fresh);
            Ok(html)
        }
    }
}

fn request(worker: &mut Worker, line: &str) -> Result<String, String> {
    writeln!(worker.stdin, "{line}").map_err(|e| format!("ssr stdin: {e}"))?;
    worker
        .stdin
        .flush()
        .map_err(|e| format!("ssr flush: {e}"))?;

    let mut response = String::new();
    worker
        .stdout
        .read_line(&mut response)
        .map_err(|e| format!("ssr stdout: {e}"))?;
    if response.is_empty() {
        return Err("ssr empty response".into());
    }

    let v: Value =
        serde_json::from_str(response.trim()).map_err(|e| format!("ssr json: {e}: {response}"))?;
    if v.get("ok").and_then(|x| x.as_bool()) == Some(true) {
        v.get("html")
            .and_then(|h| h.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "ssr missing html".into())
    } else {
        Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("ssr failed")
            .to_string())
    }
}

fn spawn_worker() -> Result<Worker, String> {
    let (cwd, script) = resolve_ssr_script().ok_or_else(|| {
        String::from(
            "missing SSR bundle — run: cd app && npm run build (needs public/build/ssr/_ssr.js)",
        )
    })?;

    let mut child = Command::new(node_bin())
        .arg(&script)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("spawn node ssr: {e}"))?;

    let stdin = child.stdin.take().ok_or("ssr stdin missing")?;
    let stdout = child.stdout.take().ok_or("ssr stdout missing")?;

    Ok(Worker {
        child,
        stdin,
        stdout: BufReader::new(stdout),
        started: Instant::now(),
    })
}

fn node_bin() -> String {
    std::env::var("NAMIX_NODE").unwrap_or_else(|_| "node".into())
}

fn resolve_ssr_script() -> Option<(PathBuf, PathBuf)> {
    let candidates = [
        PathBuf::from("public/build/ssr/_ssr.js"),
        PathBuf::from("app/public/build/ssr/_ssr.js"),
    ];
    for rel in candidates {
        if rel.is_file() {
            let cwd = if rel.starts_with("app/") {
                PathBuf::from("app")
            } else {
                PathBuf::from(".")
            };
            let script = if rel.starts_with("app/") {
                PathBuf::from("public/build/ssr/_ssr.js")
            } else {
                rel.clone()
            };
            return Some((cwd, script));
        }
    }
    // also accept any single .js under ssr/
    for dir in [
        Path::new("public/build/ssr"),
        Path::new("app/public/build/ssr"),
    ] {
        if !dir.is_dir() {
            continue;
        }
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("js") {
                    let cwd = if dir.starts_with("app") {
                        PathBuf::from("app")
                    } else {
                        PathBuf::from(".")
                    };
                    let script = PathBuf::from("public/build/ssr").join(p.file_name()?);
                    return Some((cwd, script));
                }
            }
        }
    }
    None
}
