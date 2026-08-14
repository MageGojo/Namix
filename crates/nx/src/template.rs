use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::frontend;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Multi,
    Single,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontendLang {
    Tsx,
    Jsx,
}

impl FrontendLang {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tsx => "tsx",
            Self::Jsx => "jsx",
        }
    }
}

/// 数据库驱动（脚手架写入 namix.toml + Cargo features）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseDriver {
    Sqlite,
    Mysql,
    Postgresql,
    /// 自填 url；Cargo 默认仍带 sqlite 以便能编过
    Custom,
}

impl DatabaseDriver {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Mysql => "mysql",
            Self::Postgresql => "postgresql",
            Self::Custom => "custom",
        }
    }

    /// 打开数据库时写入 Cargo `namix` features 用（lean 默认不启用）。
    #[allow(dead_code)]
    pub fn cargo_feature(self) -> &'static str {
        match self {
            Self::Sqlite | Self::Custom => "sqlite",
            Self::Mysql => "mysql",
            Self::Postgresql => "postgresql",
        }
    }

    pub fn default_url(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite:./storage/namix.db",
            Self::Mysql => "mysql://root@127.0.0.1:3306/namix",
            Self::Postgresql => "postgresql://postgres@127.0.0.1:5432/namix",
            Self::Custom => "postgresql://user:pass@127.0.0.1:5432/namix",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ScaffoldConfig {
    pub mode: Mode,
    pub https: bool,
    pub database: DatabaseDriver,
    pub frontend: FrontendLang,
    pub tailwind: bool,
    pub git: bool,
}

fn framework_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn scaffold(root: &Path, name: &str, cfg: &ScaffoldConfig) -> Result<(), String> {
    if root.exists() {
        return Err(format!("目录已存在: {}", root.display()));
    }
    fs::create_dir_all(root).map_err(|e| e.to_string())?;

    let fw = framework_root();
    let fw = fw.display().to_string().replace('\\', "/");
    let fe = cfg.frontend.label();
    let tw = if cfg.tailwind { "on" } else { "off" };

    write(
        root.join("Cargo.toml"),
        &format!(
            r#"[workspace]
resolver = "3"
members = ["app"]

[workspace.package]
version = "0.1.0"
edition = "2024"

[workspace.dependencies]
namix = {{ path = "{fw}/crates/namix" }}
namix-http = {{ path = "{fw}/crates/namix-http" }}
namix-macros = {{ path = "{fw}/crates/namix-macros" }}
namix-build = {{ path = "{fw}/crates/namix-build" }}
tokio = {{ version = "1", features = ["macros", "rt-multi-thread"] }}

# nx build 默认 profile：优先体积
[profile.release-min]
inherits = "release"
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
"#
        ),
    )?;

    write(
        root.join("README.md"),
        &format!(
            r#"# {name}

`nx new` 生成 · 框架 path → `{fw}`

**默认 lean 全栈**：`controllers` + `routes` + `app/src/views`（`[features].pages = true`）。
数据库、Model、Service、Validator、Seeder、Event 按需打开，见仓库 `docs/FEATURES.md`。

视图工具链在 **`app/`**（Vite · {fe} · tailwind={tw}），不是独立 `frontend/` 工程。

```bash
# 视图依赖
cd app && npm i && npm run build

# 后端（单应用）
cargo run -p app -- -p 3000

# 后端（多应用）
cargo run -p app --bin www -- -p 3000

# 开发：nx dev（Rust + Vite HMR）
nx doctor
```

启动后端后会写出 `app/storage/routes.*`；`nx export routes` 同步到 `app/src/views/routes.ts`。

| 参数 | 说明 |
|------|------|
| `-p` / `--port` | 端口 |
| `-h` / `--lan` | 局域网访问（0.0.0.0） |
| `--https` | 本地自签 HTTPS |
| `--https-port` | HTTPS 端口 |
"#
        ),
    )?;

    write_editor_dx(root)?;

    match cfg.mode {
        Mode::Multi => scaffold_multi(root, cfg.https, cfg.database)?,
        Mode::Single => scaffold_single(root, cfg.https, cfg.database)?,
    }

    frontend::scaffold(root, name, cfg.frontend, cfg.tailwind)?;

    write(
        root.join(".gitignore"),
        "/target/\napp/node_modules/\napp/public/build/\napp/storage/*.db\n*.db\n.DS_Store\n",
    )?;

    if cfg.git {
        git_init(root)?;
    }

    Ok(())
}

fn git_init(root: &Path) -> Result<(), String> {
    let status = Command::new("git")
        .args(["-c", "init.defaultBranch=main", "init", "-q"])
        .current_dir(root)
        .status()
        .map_err(|e| format!("git init 失败（未安装 git?）: {e}"))?;
    if !status.success() {
        return Err(format!("git init 退出码 {}", status.code().unwrap_or(-1)));
    }
    Ok(())
}

fn scaffold_multi(root: &Path, https: bool, db: DatabaseDriver) -> Result<(), String> {
    let app = root.join("app");
    fs::create_dir_all(app.join("src/bin")).map_err(|e| e.to_string())?;
    let https = if https { "true" } else { "false" };
    let driver = db.label();
    let url = db.default_url();

    write(
        app.join("Cargo.toml"),
        r#"[package]
name = "app"
version.workspace = true
edition.workspace = true
build = "build.rs"
description = "Namix 多应用（默认：controllers + routes + views）"

[[bin]]
name = "www"
path = "src/bin/www.rs"

[[bin]]
name = "user"
path = "src/bin/user.rs"

[[bin]]
name = "admin"
path = "src/bin/admin.rs"

[dependencies]
namix = { workspace = true, features = ["pages"] }
tokio = { workspace = true }
serde = { version = "1", features = ["derive"] }

[build-dependencies]
namix-build = { workspace = true }
"#,
    )?;
    write(
        app.join("build.rs"),
        "fn main() {\n    namix_build::sync();\n}\n",
    )?;
    write(
        app.join("namix.toml"),
        &format!(
            r#"# 默认脚手架：各端 controllers + routes + 共享 views（pages）。
# 打开 models/services/validators/seeders 后请同步 Cargo features（如 {driver}）。

[database]
enabled = false
driver = "{driver}"
url = "{url}"
push_schema = true

[features]
models = false
services = false
validators = false
requests = false
pages = true
events = false
listeners = false
seeders = false
action_seal = true

[i18n]
locale = "zh-CN"
path = "./lang"

[apps.www]
hosts = ["www.localhost"]
port = 3000
https = {https}
https_port = 3443
http3 = false
lan = false

[apps.user]
hosts = ["user.localhost"]
port = 3001
https = {https}
https_port = 3444
http3 = false
lan = false

[apps.admin]
hosts = ["admin.localhost"]
port = 3002
https = {https}
https_port = 3445
http3 = false
lan = false

[security]
environment = "development"
csrf = true
"#
        ),
    )?;
    write(
        app.join("src/lib.rs"),
        "//! 多应用业务包。\n\
         //! 默认：www|user|admin 的 controllers/routes/middleware；views 由 pages feature 管理。\n\
         pub mod prelude;\n\
         pub mod admin;\n\
         pub mod common;\n\
         pub mod route;\n\
         pub mod user;\n\
         pub mod view;\n\
         pub mod www;\n",
    )?;
    write(
        app.join("src/prelude.rs"),
        "//! 业务侧一键导入。多应用下命名路由在 `route::user::login` / `route::user::AppRoute`。\n\
         //!\n\
         //! `Route` 仍是注册路由用的 `Route::get`。\n\n\
         pub use namix::prelude::*;\n\
         pub use crate::route;\n\
         pub use crate::view::{self, Page};\n",
    )?;

    for (path, body) in [
        (
            "src/route.rs",
            r#"//! 自动生成命名路由：`route::user::login` / `route::user::AppRoute::Login`
include!(concat!(env!("OUT_DIR"), "/namix_route_names.rs"));
"#,
        ),
        ("src/view.rs", VIEW_RS_STUB),
        (
            "src/common/middleware/logger.rs",
            r#"use std::time::Instant;
use namix::prelude::*;

pub async fn logger(req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.path().to_string();
    let started = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let ms = started.elapsed().as_millis();
    let m = log::color_method(&method);
    let s = log::color_status(status);
    log::info!("{m} {path} → {s} ({ms}ms)");
    response
}
"#,
        ),
        (
            "src/admin/middleware/auth.rs",
            r#"use namix::http::StatusCode;
use namix::prelude::*;

pub async fn require_admin(req: Request, next: Next) -> Response {
    let ok = req.headers().get("x-admin-token").and_then(|v| v.to_str().ok()) == Some("namix-dev");
    if ok {
        return next.run(req).await;
    }
    namix::controller::with_status(StatusCode::UNAUTHORIZED, "unauthorized: X-Admin-Token: namix-dev")
}
"#,
        ),
        (
            "src/user/middleware/auth.rs",
            r#"use namix::prelude::*;
use crate::route;

pub async fn require_login(req: Request, next: Next) -> Response {
    let ok = req.bearer().is_some();
    if ok {
        return next.run(req).await;
    }
    req.redirect_guest_to(route::user::login)
}
"#,
        ),
        (
            "src/www/controllers/home.rs",
            r#"use namix::prelude::*;

pub async fn index(_req: Request) -> Response {
    html(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\" /><title>www · Namix</title></head>\
         <body style=\"font-family:system-ui;padding:2rem\"><h1>www</h1>\
         <p>默认：controllers · routes · views。打开数据库见 docs/FEATURES.md。</p></body></html>",
    )
}
"#,
        ),
        (
            "src/www/routes/web.rs",
            r#"use namix::prelude::*;
use crate::www::controllers::home;
use crate::route;

pub fn routes() -> Router {
    Router::new().merge(
        Route::get("/", home::index)
            .name(route::www::home)
            .register(),
    )
}
"#,
        ),
        (
            "src/user/controllers/home.rs",
            r#"use namix::prelude::*;

pub async fn index(_req: Request) -> Response {
    text("user app")
}

pub async fn login(_req: Request) -> Response {
    html("<h1>Login</h1><p>开启 session / models 后再接真实登录。</p>")
}

pub async fn profile(_req: Request) -> Response {
    text("user profile")
}
"#,
        ),
        (
            "src/user/routes/web.rs",
            r#"use namix::prelude::*;
use crate::route;
use crate::user::controllers::home;
use crate::user::middleware::auth::require_login;

pub fn routes() -> Router {
    Router::new()
        .merge(Route::get("/", home::index).name(route::user::home).register())
        .merge(Route::get("/login", home::login).name(route::user::login).register())
        .merge(
            Route::get("/profile", home::profile)
                .middleware(require_login)
                .name(route::user::profile)
                .register(),
        )
}
"#,
        ),
        (
            "src/admin/controllers/home.rs",
            r#"use namix::prelude::*;

pub async fn index(_req: Request) -> Response {
    text("admin ok")
}

pub async fn dashboard(_req: Request) -> Response {
    text("admin dashboard")
}
"#,
        ),
        (
            "src/admin/routes/web.rs",
            r#"use namix::prelude::*;
use crate::admin::controllers::home;
use crate::admin::middleware::auth::require_admin;
use crate::route;

pub fn routes() -> Router {
    Router::new()
        .merge(Route::get("/", home::index).name(route::admin::home).register())
        .merge(
            Route::get("/dashboard", home::dashboard)
                .middleware(require_admin)
                .name(route::admin::dashboard)
                .register(),
        )
}
"#,
        ),
        (
            "src/views/pages/home.tsx",
            r#"import { Head } from '../namix'

export default function Home({ title }: { title?: string }) {
  return (
    <main className="min-h-screen px-6 py-14">
      <Head title={title ? `${title} · Namix` : 'Namix'} />
      <h1 className="text-3xl font-semibold">{title ?? 'Namix'}</h1>
      <p className="mt-3 text-zinc-600">多应用默认脚手架：controllers · routes · views</p>
    </main>
  )
}
"#,
        ),
        (
            "src/views/.namix-feature",
            "feature = \"pages\"\n# managed by namix-build — do not remove\n",
        ),
    ] {
        write(app.join(path), body)?;
    }

    for bin in ["www", "user", "admin"] {
        write(
            app.join(format!("src/bin/{bin}.rs")),
            &format!(
                r#"use namix::Boot;

#[tokio::main]
async fn main() {{
    namix::init_workdir(env!("CARGO_MANIFEST_DIR"));
    Boot::new("{bin}")
        .toml(include_str!("../../namix.toml"))
        .middleware(app::common::middleware::logger::logger)
        .routes(app::{bin}::routes::web::routes())
        .run()
        .await
        .expect("{bin} failed");
}}
"#
            ),
        )?;
    }

    for dir in [
        "storage",
        "src/www/middleware",
        "src/user/middleware",
        "src/admin/middleware",
    ] {
        fs::create_dir_all(app.join(dir)).map_err(|e| e.to_string())?;
    }
    write(app.join("storage/.gitkeep"), "")?;
    Ok(())
}

fn scaffold_single(root: &Path, https: bool, db: DatabaseDriver) -> Result<(), String> {
    let app = root.join("app");
    fs::create_dir_all(app.join("src")).map_err(|e| e.to_string())?;
    let https = if https { "true" } else { "false" };
    let driver = db.label();
    let url = db.default_url();

    write(
        app.join("Cargo.toml"),
        r#"[package]
name = "app"
version.workspace = true
edition.workspace = true
build = "build.rs"
description = "Namix 单应用（默认：controllers + routes + views）"

[[bin]]
name = "app"
path = "src/main.rs"

[dependencies]
namix = { workspace = true, features = ["pages"] }
tokio = { workspace = true }
serde = { version = "1", features = ["derive"] }

[build-dependencies]
namix-build = { workspace = true }
"#,
    )?;
    write(
        app.join("build.rs"),
        "fn main() {\n    namix_build::sync_single();\n}\n",
    )?;
    write(
        app.join("namix.toml"),
        &format!(
            r#"# 默认脚手架：controllers + routes + views（pages）。
# 打开 models/services/validators 后请同步 Cargo features（如 {driver}）与依赖。

[database]
enabled = false
driver = "{driver}"
url = "{url}"
push_schema = true

[features]
models = false
services = false
validators = false
requests = false
pages = true
events = false
listeners = false
seeders = false
action_seal = true

[i18n]
locale = "zh-CN"
path = "./lang"

[apps.main]
hosts = ["localhost", "127.0.0.1"]
port = 3000
https = {https}
https_port = 3443
http3 = false
lan = false

[security]
environment = "development"
csrf = true
"#
        ),
    )?;
    write(
        app.join("src/lib.rs"),
        "//! 单应用业务包。\n\
         //! 默认模块：controllers / routes / middleware；views 由 pages feature 管理。\n\
         include!(\"namix_modules.rs\");\n\
         pub mod prelude;\n\
         pub mod route;\n\
         pub mod view;\n",
    )?;
    write(
        app.join("src/prelude.rs"),
        "//! 业务侧一键导入：框架 prelude + 本应用的 `AppRoute` / `Page`。\n\
         //!\n\
         //! `Route` 仍是注册路由用的 `Route::get`；命名路由枚举叫 [`AppRoute`]，避免撞名。\n\n\
         pub use namix::prelude::*;\n\
         pub use crate::route::{self, AppRoute};\n\
         pub use crate::view::{self, Page};\n",
    )?;
    write(
        app.join("src/route.rs"),
        "//! 类型化路由名（自动生成，勿手改）。\n\
         //!\n\
         //! - 注册：`name: \"home\"`\n\
         //! - 使用：`AppRoute::Home` 或 `route::main::home`\n\
         include!(concat!(env!(\"OUT_DIR\"), \"/namix_route_names.rs\"));\n",
    )?;
    write(app.join("src/view.rs"), VIEW_RS_STUB)?;
    write(
        app.join("src/main.rs"),
        r#"use namix::Boot;

#[tokio::main]
async fn main() {
    namix::init_workdir(env!("CARGO_MANIFEST_DIR"));

    Boot::new("main")
        .toml(include_str!("../namix.toml"))
        .middleware(app::middleware::logger::logger)
        .routes(app::routes::web::routes())
        .run()
        .await
        .expect("app failed");
}
"#,
    )?;

    for (path, body) in [
        (
            "src/controllers/home.rs",
            r#"use namix::prelude::*;

/// 默认首页：纯 HTML，不依赖 Vite SSR 产物。
/// 需要 `req.view` 时：保持 pages=true，在 app/ 执行 npm run build，再改用 ViewData。
pub async fn index(_req: Request) -> Response {
    html(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\" />         <title>Namix</title></head><body style=\"font-family:system-ui;padding:2rem\">         <h1>Namix</h1>         <p>默认脚手架：<b>controllers</b> · <b>routes</b> · <b>views</b></p>         <p>打开数据库 / 校验 / 会话：编辑 <code>namix.toml [features]</code> 与 Cargo features，见文档 FEATURES.md。</p>         </body></html>",
    )
}
"#,
        ),
        (
            "src/middleware/logger.rs",
            r#"use std::time::Instant;
use namix::prelude::*;

pub async fn logger(req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.path().to_string();
    let started = Instant::now();
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let ms = started.elapsed().as_millis();
    let m = log::color_method(&method);
    let s = log::color_status(status);
    log::info!("{m} {path} → {s} ({ms}ms)");
    response
}
"#,
        ),
        (
            "src/routes/web.rs",
            r#"use namix::prelude::*;
use crate::controllers::home;
use crate::route;

pub fn routes() -> Router {
    Router::new().merge(
        Route::get("/", home::index)
            .name(route::main::home)
            .register(),
    )
}
"#,
        ),
        (
            "src/views/.namix-feature",
            "feature = \"pages\"\n# managed by namix-build — do not remove\n",
        ),
    ] {
        write(app.join(path), body)?;
    }

    fs::create_dir_all(app.join("storage")).map_err(|e| e.to_string())?;
    write(app.join("storage/.gitkeep"), "")?;
    Ok(())
}

const VIEW_RS_STUB: &str = r#"// @generated by namix-build — DO NOT EDIT
//! 页面名：`req.view(Page::Home)` / `req.view(view::home)` 与 `views/pages/home.tsx` 对齐。

#![allow(non_upper_case_globals)]

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Page {
    Home,
}

impl Page {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Home => "home",
        }
    }
}

impl From<Page> for String {
    fn from(page: Page) -> Self {
        page.as_str().into()
    }
}

impl AsRef<str> for Page {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

pub const home: &str = "home";
"#;

fn write_editor_dx(root: &Path) -> Result<(), String> {
    write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"stable\"\ncomponents = [\"rustfmt\", \"clippy\"]\n",
    )?;
    write(
        root.join("rust-analyzer.toml"),
        r#"[check]
command = "clippy"
allTargets = true
workspace = false

[procMacro]
enable = true
"#,
    )?;
    write(
        root.join(".vscode/settings.json"),
        r#"{
  "rust-analyzer.check.command": "clippy",
  "rust-analyzer.check.workspace": false,
  "editor.formatOnSave": true,
  "[rust]": {
    "editor.defaultFormatter": "rust-lang.rust-analyzer"
  }
}
"#,
    )?;
    write(
        root.join(".vscode/extensions.json"),
        r#"{
  "recommendations": ["rust-lang.rust-analyzer"]
}
"#,
    )
}

fn write(path: impl AsRef<Path>, body: &str) -> Result<(), String> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, body).map_err(|e| e.to_string())
}
