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
"#
        ),
    )?;

    write(
        root.join("README.md"),
        &format!(
            r#"# {name}

`nx new` 生成 · 框架 path → `{fw}`

前端：`frontend/`（{fe} · tailwind={tw}）

```bash
# 后端
cargo run -p app --bin www -- -p 3000 -h --https   # 多应用
# 或
cargo run -p app -- -p 3000 -h --https             # 单应用

# 前端
cd frontend && npm install && npm run dev
```

启动后端后会写出 `app/storage/routes.*`；再执行 `nx export routes` 同步到 `frontend/src/routes.*`。

| 参数 | 说明 |
|------|------|
| `-p` / `--port` | 端口 |
| `-h` / `--lan` | 局域网访问（0.0.0.0） |
| `--https` | 本地自签 HTTPS |
| `--https-port` | HTTPS 端口 |
"#
        ),
    )?;

    match cfg.mode {
        Mode::Multi => scaffold_multi(root, cfg.https, cfg.database)?,
        Mode::Single => scaffold_single(root, cfg.https, cfg.database)?,
    }

    frontend::scaffold(root, name, cfg.frontend, cfg.tailwind)?;

    write(
        root.join(".gitignore"),
        "/target/\n/frontend/node_modules/\n/frontend/dist/\napp/storage/*.db\n*.db\n.DS_Store\n",
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

fn database_toml(db: DatabaseDriver, rest: &str) -> String {
    let driver = db.label();
    let url = db.default_url();
    let note = match db {
        DatabaseDriver::Custom => {
            "# custom：请改 url，并在 Cargo.toml 将 namix/toasty feature 换成 mysql 或 postgresql\n"
        }
        DatabaseDriver::Mysql => "# MySQL：确保本机已建库；Cargo feature = mysql\n",
        DatabaseDriver::Postgresql => "# PostgreSQL：确保本机已建库；Cargo feature = postgresql\n",
        DatabaseDriver::Sqlite => "# SQLite：文件库，默认 ./storage/namix.db\n",
    };
    format!(
        r#"{note}[database]
enabled = true
driver = "{driver}"
url = "{url}"
push_schema = true
{rest}"#
    )
}

fn scaffold_multi(root: &Path, https: bool, db: DatabaseDriver) -> Result<(), String> {
    let app = root.join("app");
    fs::create_dir_all(app.join("src/bin")).map_err(|e| e.to_string())?;
    let feat = db.cargo_feature();

    write(
        app.join("Cargo.toml"),
        &format!(
            r#"[package]
name = "app"
version.workspace = true
edition.workspace = true
build = "build.rs"

[[bin]]
name = "www"
path = "src/bin/www.rs"

[[bin]]
name = "user"
path = "src/bin/user.rs"

[[bin]]
name = "admin"
path = "src/bin/admin.rs"

[[bin]]
name = "seed"
path = "src/bin/seed.rs"

[dependencies]
namix = {{ workspace = true, features = ["{feat}"] }}
tokio = {{ workspace = true }}
toasty = {{ version = "0.9", default-features = false, features = ["{feat}", "serde"] }}

[build-dependencies]
namix-build = {{ workspace = true }}
"#
        ),
    )?;
    write(
        app.join("build.rs"),
        "fn main() {\n    namix_build::sync();\n}\n",
    )?;
    let https = if https { "true" } else { "false" };
    write(
        app.join("namix.toml"),
        &database_toml(
            db,
            &format!(
                r#"
[features]
validators = true
requests = false
pages = false
models = true

[apps.www]
hosts = ["www.localhost"]
port = 3000
https = {https}
https_port = 3443
http3 = true
lan = false

[apps.user]
hosts = ["user.localhost"]
port = 3001
https = {https}
https_port = 3444
http3 = true
lan = false

[apps.admin]
hosts = ["admin.localhost"]
port = 3002
https = {https}
https_port = 3445
http3 = true
lan = false
"#
            ),
        ),
    )?;
    write(
        app.join("Toasty.toml"),
        "[migration]\npath = \"database\"\nprefix_style = \"Sequential\"\n",
    )?;
    write(
        app.join("src/lib.rs"),
        "pub mod admin;\npub mod common;\npub mod route;\npub mod user;\npub mod www;\n",
    )?;

    for (path, body) in [
        (
            "src/route.rs",
            r#"//! 自动生成：业务写 `.name(route::user::login)` 即可
include!(concat!(env!("OUT_DIR"), "/namix_route_names.rs"));
"#,
        ),
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
            "src/common/middleware/timing.rs",
            r#"use std::time::Instant;
use namix::prelude::*;

pub async fn timing(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.path().to_string();
    let started = Instant::now();
    let response = next.run(req).await;
    println!("[timing] {method} {path} -> {} ({}ms)", response.status(), started.elapsed().as_millis());
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
            "src/common/models/user.rs",
            r#"#[derive(Debug, Clone, toasty::Model)]
pub struct User {
    #[key]
    #[auto]
    pub id: u64,
    pub name: String,
    #[unique]
    pub email: String,
}
"#,
        ),
        (
            "src/common/models/registry.rs",
            r#"use super::user::User;

pub fn model_set() -> toasty::ModelSet {
    toasty::models!(User)
}
"#,
        ),
        (
            "src/common/services/user.rs",
            r#"use namix::db::{self, DbResult};
use crate::common::models::user::User;

pub struct UserService;
impl UserService {
    pub async fn list() -> Vec<User> {
        db::with(|mut db| async move { toasty::query!(User).exec(&mut db).await })
            .await
            .unwrap_or_default()
    }
    pub async fn create(name: &str, email: &str) -> DbResult<User> {
        let name = name.to_string();
        let email = email.to_string();
        db::with(move |mut db| {
            let name = name.clone();
            let email = email.clone();
            async move {
                toasty::create!(User {
                    name: name.as_str(),
                    email: email.as_str(),
                })
                .exec(&mut db)
                .await
            }
        })
        .await
    }
}
"#,
        ),
        (
            "src/common/seeders/all.rs",
            r#"use namix::db::DbResult;
use super::users::UsersSeeder;

pub async fn run() -> DbResult<()> {
    UsersSeeder::run().await
}
"#,
        ),
        (
            "src/common/seeders/users.rs",
            r#"use namix::db::DbResult;
use crate::common::services::user::UserService;

pub struct UsersSeeder;
impl UsersSeeder {
    pub async fn run() -> DbResult<()> {
        if !UserService::list().await.is_empty() {
            return Ok(());
        }
        UserService::create("alice", "alice@example.com").await?;
        Ok(())
    }
}
"#,
        ),
        (
            "src/www/controllers/home.rs",
            r#"use namix::prelude::*;
use crate::common::services::user::UserService;

pub async fn index(_req: Request) -> Response {
    text(format!("www — {} users", UserService::list().await.len()))
}
"#,
        ),
        (
            "src/www/routes/web.rs",
            r#"use namix::prelude::*;
use crate::www::controllers::home;

pub fn routes() -> Router {
    Router::new().get("/", home::index)
}
"#,
        ),
        (
            "src/common/events/user_registered.rs",
            r#"//! 注册页 dispatch → 各功能 listen → Outcome 汇总回注册页。

#[derive(Clone, Debug)]
pub struct UserRegistered {
    pub username: String,
}
"#,
        ),
        (
            "src/common/listeners/register.rs",
            r#"use namix::prelude::*;
use crate::common::events::user_registered::UserRegistered;

/// 启动时挂监听器。
pub fn all() {
    listen(|e: &UserRegistered| {
        Reply::ok(format!("account created · {}", e.username))
    });
    listen(|e: &UserRegistered| {
        Reply::ok(format!("welcome mail → {}", e.username))
    });
}
"#,
        ),
        (
            "src/common/validators/register_form.rs",
            r#"//! 表单验证器：字段 enum + Rule。features.validators = true 时自动保留本目录。

use namix::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum RegisterForm {
    #[field = "username"]
    Username,
}

pub fn validate(req: &Request) -> Result<Validated, ValidationError> {
    req.validator()
        .rules(RegisterForm::Username, &[Rule::Required, Rule::Min(3)])
        .validate()
}
"#,
        ),
        (
            "src/user/controllers/home.rs",
            r#"use namix::prelude::*;
use crate::common::events::user_registered::UserRegistered;
use crate::common::validators::register_form::{self, RegisterForm};

pub async fn index(_req: Request) -> Response {
    text("user app")
}

pub async fn register(_req: Request) -> Response {
    html(
        "<h1>Register</h1>\
         <form method=\"post\" action=\"/register\">\
           <input name=\"username\" placeholder=\"username\" />\
           <button type=\"submit\">Sign up</button>\
         </form>\
         <p>规则在 common/validators · 提交后 Event 回执</p>",
    )
}

pub async fn register_submit(req: Request) -> Response {
    let validated = match register_form::validate(&req) {
        Ok(v) => v,
        Err(_) => return Response::redirect("/register"),
    };
    let username = validated.get(RegisterForm::Username).to_string();
    let outcome = dispatch(UserRegistered {
        username: username.clone(),
    });
    let steps = outcome
        .messages()
        .into_iter()
        .map(|m| format!("<li>{m}</li>"))
        .collect::<String>();
    html(format!(
        "<h1>注册成功</h1><p>{username}</p><ul>{steps}</ul>"
    ))
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
        .merge(Route::get("/login", home::index).name(route::user::login).register())
        .merge(Route::get("/register", home::register).name(route::user::register).register())
        .merge(Route::post("/register", home::register_submit).name(route::user::register).register())
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
use crate::common::services::user::UserService;

pub async fn index(_req: Request) -> Response {
    text("admin ok")
}

pub async fn users(_req: Request) -> Response {
    let names: Vec<_> = UserService::list().await.into_iter().map(|u| u.name).collect();
    text(format!("admin users: {}", names.join(", ")))
}
"#,
        ),
        (
            "src/admin/routes/web.rs",
            r#"use namix::prelude::*;
use crate::admin::controllers::home;
use crate::admin::middleware::auth::require_admin;

pub fn routes() -> Router {
    Router::new()
        .get("/", home::index)
        .group("/users", |r| {
            r.get("/", home::users).middleware(require_admin)
        })
}
"#,
        ),
    ] {
        write(app.join(path), body)?;
    }

    for bin in ["www", "admin"] {
        write(
            app.join(format!("src/bin/{bin}.rs")),
            &format!(
                r#"use namix::Boot;

#[tokio::main]
async fn main() {{
    namix::init_workdir(env!("CARGO_MANIFEST_DIR"));
    Boot::new("{bin}")
        .toml(include_str!("../../namix.toml"))
        .models(app::common::models::registry::model_set())
        .middleware(app::common::middleware::logger::logger)
        .middleware(app::common::middleware::timing::timing)
        .routes(app::{bin}::routes::web::routes())
        .run()
        .await
        .expect("{bin} failed");
}}
"#
            ),
        )?;
    }
    write(
        app.join("src/bin/user.rs"),
        r#"use namix::Boot;

#[tokio::main]
async fn main() {
    namix::init_workdir(env!("CARGO_MANIFEST_DIR"));
    app::common::listeners::register::all();

    Boot::new("user")
        .toml(include_str!("../../namix.toml"))
        .models(app::common::models::registry::model_set())
        .middleware(app::common::middleware::logger::logger)
        .middleware(app::common::middleware::timing::timing)
        .routes(app::user::routes::web::routes())
        .run()
        .await
        .expect("user failed");
}
"#,
    )?;
    write(
        app.join("src/bin/seed.rs"),
        r#"use namix::{db, NamixToml};

#[tokio::main]
async fn main() {
    let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
    namix::log::init();
    let cfg = NamixToml::parse(include_str!("../../namix.toml"));
    let db = db::connect(&cfg.database.resolved_url(), app::common::models::registry::model_set())
        .await
        .expect("connect");
    if cfg.database.push_schema {
        db.push_schema().await.expect("schema");
    }
    db::install(db);
    app::common::seeders::all::run().await.expect("seed");
}
"#,
    )?;

    for dir in [
        "src/common/models",
        "src/common/events",
        "src/common/listeners",
        "src/common/validators",
        "src/common/seeders",
        "database/migrations",
        "storage",
        "src/www/middleware",
        "src/user/middleware",
        "src/admin/middleware",
    ] {
        fs::create_dir_all(app.join(dir)).map_err(|e| e.to_string())?;
    }
    // feature 目录标记：namix-build 关闭 feature 时只删带此标记的目录
    write(
        app.join("src/common/validators/.namix-feature"),
        "feature = \"validators\"\n# managed by namix-build — do not remove\n",
    )?;
    Ok(())
}

fn scaffold_single(root: &Path, https: bool, db: DatabaseDriver) -> Result<(), String> {
    let app = root.join("app");
    fs::create_dir_all(app.join("src/bin")).map_err(|e| e.to_string())?;
    let feat = db.cargo_feature();

    write(
        app.join("Cargo.toml"),
        &format!(
            r#"[package]
name = "app"
version.workspace = true
edition.workspace = true
build = "build.rs"
description = "Namix 单应用：扁平 MVC + models/services"

[[bin]]
name = "app"
path = "src/main.rs"

[[bin]]
name = "seed"
path = "src/bin/seed.rs"

[[bin]]
name = "toasty"
path = "src/bin/toasty.rs"

[dependencies]
namix = {{ workspace = true, features = ["{feat}"] }}
tokio = {{ workspace = true }}
toasty = {{ version = "0.9", default-features = false, features = ["{feat}", "serde"] }}
toasty-cli = "0.9"

[build-dependencies]
namix-build = {{ workspace = true }}
"#
        ),
    )?;
    write(
        app.join("build.rs"),
        "fn main() {\n    namix_build::sync_single();\n}\n",
    )?;
    let https = if https { "true" } else { "false" };
    write(
        app.join("namix.toml"),
        &database_toml(
            db,
            &format!(
                r#"
[features]
validators = true
requests = false
pages = false

[apps.main]
hosts = ["localhost"]
port = 3000
https = {https}
https_port = 3443
http3 = true
lan = false
"#
            ),
        ),
    )?;
    write(
        app.join("Toasty.toml"),
        "[migration]\npath = \"database\"\nprefix_style = \"Sequential\"\n",
    )?;
    write(
        app.join("src/lib.rs"),
        "//! 单应用业务包（扁平目录）。\n\
         //!\n\
         //! - models / services / seeders — 数据层\n\
         //! - controllers / routes / middleware — HTTP 层\n\
         include!(\"namix_modules.rs\");\n\
         pub mod route;\n",
    )?;
    write(
        app.join("src/route.rs"),
        "//! 自动生成：业务写 `.name(route::main::home)` 即可\n\
         include!(concat!(env!(\"OUT_DIR\"), \"/namix_route_names.rs\"));\n",
    )?;
    write(
        app.join("src/main.rs"),
        r#"use namix::Boot;

#[tokio::main]
async fn main() {
    namix::init_workdir(env!("CARGO_MANIFEST_DIR"));

    Boot::new("main")
        .toml(include_str!("../namix.toml"))
        .models(app::models::registry::model_set())
        .middleware(app::middleware::logger::logger)
        .routes(app::routes::web::routes())
        .run()
        .await
        .expect("app failed");
}
"#,
    )?;
    write(
        app.join("src/bin/seed.rs"),
        r#"use namix::{db, NamixToml};

#[tokio::main]
async fn main() {
    let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
    namix::log::init();
    let cfg = NamixToml::parse(include_str!("../../namix.toml"));
    let db = db::connect(
        &cfg.database.resolved_url(),
        app::models::registry::model_set(),
    )
    .await
    .expect("connect");
    if cfg.database.push_schema {
        let _ = db.push_schema().await;
    }
    db::install(db);
    app::seeders::all::run().await.expect("seed");
    namix::log::info!("seed complete");
}
"#,
    )?;
    write(
        app.join("src/bin/toasty.rs"),
        r#"use std::path::Path;
use namix::db;
use toasty_cli::{Config, MigrationConfig, ToastyCli};

#[tokio::main]
async fn main() {
    let _ = std::env::set_current_dir(env!("CARGO_MANIFEST_DIR"));
    namix::log::init();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:./storage/namix.db".into());
    let db = db::connect(&url, app::models::registry::model_set())
        .await
        .expect("connect");
    let config = Config::load_from(Path::new("Toasty.toml")).unwrap_or_else(|_| {
        Config::new().migration(MigrationConfig::new().path("database"))
    });
    ToastyCli::with_config(db, config)
        .parse_and_run()
        .await
        .expect("toasty failed");
}
"#,
    )?;

    for (path, body) in [
        (
            "src/models/user.rs",
            r#"#[derive(Debug, Clone, toasty::Model)]
pub struct User {
    #[key]
    #[auto]
    pub id: u64,
    pub name: String,
    #[unique]
    pub email: String,
}
"#,
        ),
        (
            "src/models/registry.rs",
            r#"use super::user::User;

pub fn model_set() -> toasty::ModelSet {
    toasty::models!(User)
}
"#,
        ),
        (
            "src/services/user.rs",
            r#"//! 碰库只写这里。

use namix::db::{self, DbResult};
use crate::models::user::User;

pub struct UserService;

impl UserService {
    pub async fn list() -> Vec<User> {
        db::with(|mut db| async move { toasty::query!(User).exec(&mut db).await })
            .await
            .unwrap_or_default()
    }

    pub async fn create(name: &str, email: &str) -> DbResult<User> {
        let name = name.to_string();
        let email = email.to_string();
        db::with(move |mut db| {
            let name = name.clone();
            let email = email.clone();
            async move {
                toasty::create!(User {
                    name: name.as_str(),
                    email: email.as_str(),
                })
                .exec(&mut db)
                .await
            }
        })
        .await
    }
}
"#,
        ),
        (
            "src/seeders/all.rs",
            r#"use namix::db::DbResult;
use super::users::UsersSeeder;

pub async fn run() -> DbResult<()> {
    UsersSeeder::run().await
}
"#,
        ),
        (
            "src/seeders/users.rs",
            r#"use namix::db::DbResult;
use crate::services::user::UserService;

pub struct UsersSeeder;

impl UsersSeeder {
    pub async fn run() -> DbResult<()> {
        if !UserService::list().await.is_empty() {
            return Ok(());
        }
        UserService::create("alice", "alice@example.com").await?;
        namix::log::info!("seeded alice");
        Ok(())
    }
}
"#,
        ),
        (
            "src/controllers/home.rs",
            r#"use namix::prelude::*;
use crate::services::user::UserService;

pub async fn index(_req: Request) -> Response {
    let n = UserService::list().await.len();
    html(format!(
        "<h1>Namix Single</h1><p>users in sqlite: <b>{n}</b></p>\
         <p>实体 → models/ · 碰库 → services/ · 本页 → controllers/</p>"
    ))
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
            "src/validators/register_form.rs",
            r#"use namix::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum RegisterForm {
    #[field = "username"]
    Username,
}

pub fn validate(req: &Request) -> Result<Validated, ValidationError> {
    req.validator()
        .rules(RegisterForm::Username, &[Rule::Required, Rule::Min(3)])
        .validate()
}
"#,
        ),
    ] {
        write(app.join(path), body)?;
    }

    for dir in [
        "src/events",
        "src/listeners",
        "database/migrations",
        "storage",
    ] {
        fs::create_dir_all(app.join(dir)).map_err(|e| e.to_string())?;
    }
    write(
        app.join("src/validators/.namix-feature"),
        "feature = \"validators\"\n# managed by namix-build — do not remove\n",
    )?;
    write(app.join("storage/.gitkeep"), "")?;
    Ok(())
}

fn write(path: impl AsRef<Path>, body: &str) -> Result<(), String> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, body).map_err(|e| e.to_string())
}
