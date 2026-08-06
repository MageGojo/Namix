//! `nx doctor` — 单/多应用自检。

use std::fs;
use std::process::{Command, Stdio};

use crate::project::Project;
use crate::template::Mode;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Ok,
    Warn,
    Fail,
}

struct Check {
    level: Level,
    title: String,
    detail: String,
}

pub fn run(project: &Project, with_compile: bool) -> Result<(), String> {
    let mut checks = Vec::new();

    let mode = match project.mode {
        Mode::Single => "single",
        Mode::Multi => "multi",
    };
    checks.push(Check {
        level: Level::Ok,
        title: format!("项目模式 = {mode}"),
        detail: format!("app = {}", project.app_dir.display()),
    });

    // namix.toml
    let toml_path = project.namix_toml();
    let toml = fs::read_to_string(&toml_path).unwrap_or_default();
    if toml.is_empty() {
        checks.push(fail("namix.toml", "无法读取"));
    } else {
        checks.push(ok("namix.toml", "存在"));
        if toml.contains("[database]") && (toml.contains("url") || toml.contains("driver")) {
            checks.push(ok("[database]", "已配置"));
        } else {
            checks.push(warn(
                "[database]",
                "缺少 [database] url/driver — 启动无法连库",
            ));
        }
        match project.mode {
            Mode::Single => {
                if toml.contains("[apps.main]") {
                    checks.push(ok("[apps.main]", "已配置"));
                } else {
                    checks.push(fail("[apps.main]", "单应用需要 [apps.main]"));
                }
                if toml.contains("[apps.www]") || toml.contains("[apps.user]") {
                    checks.push(warn(
                        "多余 apps.*",
                        "单应用一般只需 [apps.main]，仍保留多端配置也可",
                    ));
                }
            }
            Mode::Multi => {
                for app in ["www", "user", "admin"] {
                    let key = format!("[apps.{app}]");
                    if toml.contains(&key) {
                        checks.push(ok(&key, "已配置"));
                    } else {
                        checks.push(warn(&key, "未配置（若不用该端可忽略）"));
                    }
                }
            }
        }
        if toml.contains("validators = true") || toml.contains("validators=true") {
            checks.push(ok("[features].validators", "开启"));
        } else {
            checks.push(warn(
                "[features].validators",
                "未开启 — nx make validator 仍可写文件，但目录可能被 sync 删掉",
            ));
        }
    }

    // build.rs ↔ 目录
    let build = fs::read_to_string(project.build_rs()).unwrap_or_default();
    match project.mode {
        Mode::Single => {
            if build.contains("sync_single") {
                checks.push(ok("build.rs", "sync_single()"));
            } else {
                checks.push(fail("build.rs", "单应用应调用 namix_build::sync_single()"));
            }
            expect_dir(
                &mut checks,
                &project.src_dir().join("controllers"),
                "src/controllers",
            );
            expect_dir(&mut checks, &project.src_dir().join("routes"), "src/routes");
            expect_dir(&mut checks, &project.models_dir(), "src/models");
            expect_dir(&mut checks, &project.services_dir(), "src/services");
            if project.src_dir().join("main.rs").is_file() {
                checks.push(ok("src/main.rs", "存在"));
            } else {
                checks.push(fail("src/main.rs", "单应用入口缺失"));
            }
            if project.src_dir().join("www").is_dir() {
                checks.push(warn(
                    "src/www",
                    "单应用下仍有多应用目录，可能是未清理的残留",
                ));
            }
        }
        Mode::Multi => {
            if build.contains("sync_single") {
                checks.push(fail(
                    "build.rs",
                    "多应用应调用 namix_build::sync()，不是 sync_single",
                ));
            } else if build.contains("sync()") || build.contains("sync(") {
                checks.push(ok("build.rs", "sync()"));
            } else {
                checks.push(fail("build.rs", "应调用 namix_build::sync()"));
            }
            expect_dir(&mut checks, &project.src_dir().join("common"), "src/common");
            for app in ["www", "user", "admin"] {
                expect_dir(
                    &mut checks,
                    &project.src_dir().join(app),
                    &format!("src/{app}"),
                );
            }
        }
    }

    // registry / toasty / bins
    if project.registry_rs().is_file() {
        let reg = fs::read_to_string(project.registry_rs()).unwrap_or_default();
        if reg.contains("toasty::models!") {
            checks.push(ok("models/registry.rs", "含 model_set()"));
        } else {
            checks.push(fail("models/registry.rs", "缺少 toasty::models!"));
        }
    } else {
        checks.push(fail(
            "models/registry.rs",
            "缺失 — Boot/seed/toasty 需要 model_set()",
        ));
    }

    if project.toasty_toml().is_file() {
        checks.push(ok("Toasty.toml", "存在"));
    } else {
        checks.push(warn("Toasty.toml", "缺失 — nx migrate 仍可跑，建议补上"));
    }

    let cargo = fs::read_to_string(project.app_dir.join("Cargo.toml")).unwrap_or_default();
    if namix_feature_enabled(&cargo, "sqlite") {
        checks.push(ok("Cargo namix/sqlite", "已启用"));
    } else if cargo.contains("namix") {
        checks.push(warn(
            "Cargo namix/sqlite",
            "未看到 features = [\"sqlite\"]，连库可能失败",
        ));
    }

    for bin in ["toasty", "seed"] {
        let path = project.app_dir.join(format!("src/bin/{bin}.rs"));
        if path.is_file() {
            checks.push(ok(&format!("bin/{bin}"), "存在"));
        } else {
            checks.push(fail(
                &format!("bin/{bin}"),
                "缺失 — 无法 nx migrate / nx seed",
            ));
        }
    }

    match project.mode {
        Mode::Single => {
            if project.src_dir().join("main.rs").is_file() {
                checks.push(ok("bin/app", "src/main.rs"));
            } else {
                checks.push(fail("bin/app", "缺少 src/main.rs"));
            }
        }
        Mode::Multi => {
            for bin in ["www", "user", "admin"] {
                if project.app_dir.join(format!("src/bin/{bin}.rs")).is_file() {
                    checks.push(ok(&format!("bin/{bin}"), "存在"));
                } else {
                    checks.push(warn(&format!("bin/{bin}"), "缺失"));
                }
            }
        }
    }

    expect_dir(&mut checks, &project.app_dir.join("storage"), "storage/");
    expect_dir(&mut checks, &project.app_dir.join("database"), "database/");
    expect_dir(&mut checks, &project.seeders_dir(), "seeders/");

    // route.rs
    if project.src_dir().join("route.rs").is_file() {
        checks.push(ok("src/route.rs", "命名路由入口"));
    } else {
        checks.push(warn("src/route.rs", "缺失 — .name(route::…) 无法编译"));
    }

    if with_compile {
        println!("→ cargo check {} …", pkg_flag(project));
        match cargo_check(project) {
            Ok(()) => checks.push(ok("cargo check", "通过")),
            Err(e) => checks.push(fail("cargo check", &e)),
        }
    }

    print_report(&checks);
    let failed = checks.iter().any(|c| c.level == Level::Fail);
    if failed {
        Err("自检未通过，请按上方 Fail 项修复".into())
    } else {
        println!("✓ doctor 通过（mode={mode}）");
        Ok(())
    }
}

fn namix_feature_enabled(cargo_manifest: &str, feature: &str) -> bool {
    let Ok(manifest) = toml::from_str::<toml::Value>(cargo_manifest) else {
        return false;
    };
    manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .and_then(|dependencies| dependencies.get("namix"))
        .and_then(toml::Value::as_table)
        .and_then(|namix| namix.get("features"))
        .and_then(toml::Value::as_array)
        .is_some_and(|features| features.iter().any(|value| value.as_str() == Some(feature)))
}

fn pkg_flag(project: &Project) -> String {
    if project.uses_workspace_package() {
        "-p app".into()
    } else {
        String::new()
    }
}

fn cargo_check(project: &Project) -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("check");
    if project.uses_workspace_package() {
        cmd.args(["-p", "app"]);
    }
    cmd.current_dir(project.cargo_cwd());
    cmd.stdin(Stdio::null());
    let out = cmd.output().map_err(|e| format!("无法执行 cargo: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        Err(if tail.is_empty() {
            "编译失败".into()
        } else {
            tail
        })
    }
}

fn expect_dir(checks: &mut Vec<Check>, path: &std::path::Path, label: &str) {
    if path.is_dir() {
        checks.push(ok(label, "目录存在"));
    } else {
        checks.push(fail(label, "目录缺失"));
    }
}

fn ok(title: &str, detail: impl Into<String>) -> Check {
    Check {
        level: Level::Ok,
        title: title.into(),
        detail: detail.into(),
    }
}
fn warn(title: &str, detail: impl Into<String>) -> Check {
    Check {
        level: Level::Warn,
        title: title.into(),
        detail: detail.into(),
    }
}
fn fail(title: &str, detail: impl Into<String>) -> Check {
    Check {
        level: Level::Fail,
        title: title.into(),
        detail: detail.into(),
    }
}

fn print_report(checks: &[Check]) {
    println!();
    println!("Namix doctor");
    println!("{}", "-".repeat(40));
    for c in checks {
        let tag = match c.level {
            Level::Ok => " OK ",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        };
        println!("[{tag}] {:<22} {}", c.title, c.detail);
    }
    println!("{}", "-".repeat(40));
}

#[cfg(test)]
mod tests {
    use super::namix_feature_enabled;

    #[test]
    fn detects_sqlite_among_multiple_namix_features() {
        let manifest = r#"
            [dependencies]
            namix = { workspace = true, features = ["sqlite", "pages"] }
        "#;
        assert!(namix_feature_enabled(manifest, "sqlite"));
        assert!(namix_feature_enabled(manifest, "pages"));
        assert!(!namix_feature_enabled(manifest, "postgresql"));
    }
}
