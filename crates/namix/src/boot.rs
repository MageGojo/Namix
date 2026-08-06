//! 短启动入口：业务 bin 只组装 routes / middleware，监听参数由 server + CLI 处理。
//!
//! ```rust,ignore
//! namix::Boot::new("www")
//!     .toml(include_str!("../../namix.toml"))
//!     .models(toasty::models!(User))
//!     .middleware(logger)
//!     .routes(routes())
//!     .run()
//!     .await
//! ```

use std::future::Future;
use std::net::IpAddr;
use std::time::Duration;

use clap::{ArgAction, Parser};

use crate::CsrfConfig;
use crate::config::{HttpsConfig, NamixToml};
use crate::controller::text;
use crate::{MiddlewareFn, Next, Request, Response, Route, Router, Server, wrap_middleware};

#[derive(Parser, Debug)]
#[command(name = "namix", about = "Namix app server", disable_help_flag = true)]
struct ServeArgs {
    #[arg(long = "help", action = ArgAction::Help)]
    help: Option<bool>,

    #[arg(short = 'p', long = "port")]
    port: Option<u16>,

    #[arg(short = 'h', long = "lan")]
    lan: bool,

    #[arg(long = "https")]
    https: bool,

    #[arg(long = "https-port")]
    https_port: Option<u16>,
}

pub struct Boot {
    app: String,
    toml: Option<&'static str>,
    router: Router,
    middlewares: Vec<MiddlewareFn>,
    #[cfg(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "turso",
        feature = "dynamodb"
    ))]
    models: Option<crate::db::ModelSet>,
}

impl Boot {
    pub fn new(app: impl Into<String>) -> Self {
        Self {
            app: app.into(),
            toml: None,
            router: Router::new(),
            middlewares: Vec::new(),
            #[cfg(any(
                feature = "sqlite",
                feature = "postgresql",
                feature = "mysql",
                feature = "turso",
                feature = "dynamodb"
            ))]
            models: None,
        }
    }

    pub fn toml(mut self, raw: &'static str) -> Self {
        self.toml = Some(raw);
        self
    }

    pub fn routes(mut self, router: Router) -> Self {
        self.router = router;
        self
    }

    pub fn middleware<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.middlewares.push(wrap_middleware(f));
        self
    }

    /// 注册 Toasty 模型（启用任一数据库 feature 时可用）。
    #[cfg(any(
        feature = "sqlite",
        feature = "postgresql",
        feature = "mysql",
        feature = "turso",
        feature = "dynamodb"
    ))]
    pub fn models(mut self, models: crate::db::ModelSet) -> Self {
        self.models = Some(models);
        self
    }

    pub async fn run(self) -> std::io::Result<()> {
        crate::log::init();
        let args = ServeArgs::parse();
        let embedded = self.toml.unwrap_or_else(|| {
            panic!("Boot 需要 .toml(include_str!(\"../namix.toml\"))");
        });
        // `NAMIX_CONFIG` points at a release-stable file in the shared data
        // plane. It keeps secrets and production topology out of immutable
        // release directories; a local `./namix.toml` remains the fallback.
        let runtime_config = std::env::var("NAMIX_CONFIG").ok();
        let file_toml = match runtime_config.as_deref() {
            Some(path) => Some(std::fs::read_to_string(path).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("read NAMIX_CONFIG {path}: {error}"),
                )
            })?),
            None => std::fs::read_to_string("namix.toml").ok(),
        };
        let cfg = if let Some(ref raw) = file_toml {
            let source = runtime_config.as_deref().unwrap_or("./namix.toml");
            crate::log::info!("config → {source} (runtime override)");
            NamixToml::try_parse(raw)
        } else {
            NamixToml::try_parse(embedded)
        }
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        cfg.validate(&self.app)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        let app_cfg = cfg.app(&self.app).clone();
        crate::config::install_session_secret(
            cfg.security.session_secret.clone(),
            cfg.is_production(),
        );
        write_pidfile();

        crate::mail::init(&cfg.mail);
        crate::sms::init(&cfg.sms);

        #[cfg(any(
            feature = "sqlite",
            feature = "postgresql",
            feature = "mysql",
            feature = "turso",
            feature = "dynamodb"
        ))]
        {
            if !cfg.database.enabled {
                crate::log::info!("database.enabled = false — skip DB connect");
            } else if let Some(models) = self.models {
                let url = cfg.database.resolved_url();
                crate::log::info!("database → {url}");
                let db = crate::db::connect(&url, models)
                    .await
                    .unwrap_or_else(|e| panic!("database connect failed: {e}"));
                if cfg.database.push_schema {
                    match db.push_schema().await {
                        Ok(()) => crate::log::info!("database schema pushed"),
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("already exists") {
                                crate::log::info!("database schema already present");
                            } else {
                                panic!("push_schema failed: {e}");
                            }
                        }
                    }
                }
                crate::db::install(db);
            }
        }

        let mut server = cfg.server_for(&self.app);

        if let Some(port) = args.port {
            server = server.port(port);
        }
        if args.lan {
            server = server.lan(true);
        }

        let hosts: Vec<&str> = if !app_cfg.tls_hosts.is_empty() {
            app_cfg.tls_hosts.iter().map(String::as_str).collect()
        } else if !app_cfg.hosts.is_empty() {
            app_cfg.hosts.iter().map(String::as_str).collect()
        } else {
            vec!["localhost", "127.0.0.1"]
        };

        if args.https {
            let http_port = args
                .port
                .or(app_cfg.port)
                .or_else(|| server.http_addr().map(|a| a.port()))
                .unwrap_or(3000);
            let https_port = args
                .https_port
                .or(app_cfg.https_port)
                .unwrap_or(http_port.saturating_add(443));
            server = server.local_https(true, https_port, &hosts);
            if args.lan || app_cfg.lan {
                server = server.lan(true);
            }
        } else if let Some(p) = args.https_port {
            server = server.https_port(p);
        }

        for mw in self.middlewares {
            server = attach_middleware(server, mw);
        }
        // First built-in layer: every downstream log/event carries a stable
        // request identifier and timing fields (tracing/OpenTelemetry bridge).
        server = attach_middleware(server, crate::request_id_middleware());

        if cfg.security.csrf {
            let csrf = CsrfConfig {
                trusted_origins: cfg.security.csrf_trusted_origins.clone(),
                secure_cookie: cfg.is_production() || !matches!(app_cfg.https, HttpsConfig::Off),
                ..CsrfConfig::default()
            };
            server = attach_middleware(server, crate::CsrfProtection::new(csrf).middleware());
        }
        if cfg.security.rate_limit.enabled {
            let window = Duration::from_secs(cfg.security.rate_limit.window_seconds);
            let limiter = crate::RateLimiter::new();
            server = attach_middleware(
                server,
                limiter
                    .clone()
                    .upload_middleware(crate::RateLimitPolicy::per_user_or_ip(
                        cfg.security.rate_limit.upload,
                        window,
                    )),
            );
            crate::server_fn::configure_rate_limits(crate::ActionRateLimits::new(
                limiter,
                crate::RateLimitPolicy::per_ip(cfg.security.rate_limit.login, window),
                crate::RateLimitPolicy::per_ip(cfg.security.rate_limit.registration, window),
                crate::RateLimitPolicy::per_user_or_ip(cfg.security.rate_limit.action, window),
            ));
        }

        // 调试：命名路由 JSON（前端也可直接读 storage/routes.json）
        let mut router = self
            .router
            .merge(
                Route::get("/__namix/health", namix_health)
                    .name("__namix.health")
                    .register(),
            )
            .merge(
                Route::get("/__namix/routes", namix_routes_json)
                    .name("__namix.routes")
                    .register(),
            );

        // #[server] → 单次 POST /api/a（公钥嵌入 WASM；action_seal 可关）
        crate::server_fn::configure(&self.app, cfg.features.action_seal);
        crate::log::info!(
            "action_seal = {} (NAMIX_ACTION_SEAL overrides toml)",
            crate::server_fn::action_seal_enabled()
        );
        router = router.merge(crate::server_fn::routes());

        #[cfg(feature = "pages")]
        {
            router = router.merge(crate::pages::routes());
        }

        let catalog = router.catalog();
        write_routes_exports(&catalog);
        server = server.routes(router);

        print_access_urls(&server, args.lan || app_cfg.lan);
        server.run().await
    }
}

async fn namix_health(_req: Request) -> Response {
    let version = std::env::var("NAMIX_RELEASE_VERSION").unwrap_or_else(|_| "development".into());
    let revision = std::env::var("NAMIX_RELEASE_REVISION").ok();
    namix_http::controller::json_raw(
        serde_json::json!({
            "status": "ok",
            "version": version,
            "revision": revision,
        })
        .to_string(),
    )
}

async fn namix_routes_json(req: Request) -> Response {
    match req.routes_json() {
        Some(body) => namix_http::controller::json_raw(body),
        None => text("{}"),
    }
}

fn write_routes_exports(catalog: &crate::RouteCatalog) {
    write_route_artifact(
        catalog,
        &["storage/routes.json", "app/storage/routes.json"],
        |c, p| c.write_json_file(p),
    );
    // TSX 优先；若项目是 JSX 脚手架则写 routes.js
    if std::path::Path::new("frontend/src/routes.js").is_file()
        || std::path::Path::new("../frontend/src/routes.js").is_file()
    {
        write_route_artifact(
            catalog,
            &[
                "frontend/src/routes.js",
                "../frontend/src/routes.js",
                "storage/routes.js",
                "app/storage/routes.js",
            ],
            |c, p| c.write_js_file(p),
        );
    } else {
        write_route_artifact(
            catalog,
            &[
                "src/views/routes.ts",
                "app/src/views/routes.ts",
                "frontend/src/routes.ts",
                "../frontend/src/routes.ts",
                "storage/routes.ts",
                "app/storage/routes.ts",
            ],
            |c, p| c.write_ts_file(p),
        );
    }
}

fn write_route_artifact(
    catalog: &crate::RouteCatalog,
    candidates: &[&str],
    write: impl Fn(&crate::RouteCatalog, &str) -> std::io::Result<()>,
) {
    for path in candidates {
        // 前端文件：仅在已有文件或父目录存在时写入，避免凭空建 frontend/
        let p = std::path::Path::new(path);
        let allow = path.contains("storage/")
            || p.is_file()
            || p.parent().map(|d| d.is_dir()).unwrap_or(false);
        if !allow {
            continue;
        }
        match write(catalog, path) {
            Ok(()) => {
                crate::log::info!("named routes → {path}");
                return;
            }
            Err(e) => crate::log::debug!("skip writing {path}: {e}"),
        }
    }
    if let Some(first) = candidates.first() {
        crate::log::warn!("could not write {first}");
    }
}

fn attach_middleware(server: Server, mw: MiddlewareFn) -> Server {
    server.middleware(move |req, next| {
        let mw = mw.clone();
        async move { mw(req, next).await }
    })
}

fn print_access_urls(server: &Server, lan: bool) {
    println!("---------- namix ready ----------");
    if let Some(addr) = server.http_addr() {
        let port = addr.port();
        println!("local  http://127.0.0.1:{port}");
        if lan || matches!(addr.ip(), IpAddr::V4(v) if v.is_unspecified()) {
            for ip in lan_ips() {
                println!("lan    http://{ip}:{port}");
            }
        }
    }
    if let Some(addr) = server.https_addr() {
        let port = addr.port();
        println!("local  https://127.0.0.1:{port}  (self-signed)");
        if lan || matches!(addr.ip(), IpAddr::V4(v) if v.is_unspecified()) {
            for ip in lan_ips() {
                println!("lan    https://{ip}:{port}  (self-signed)");
            }
        }
    }
    println!("routes GET /__namix/routes  (named route JSON for frontend)");
    println!("flags  -p/--port  -h/--lan  --https  --https-port  --help");
    println!("---------------------------------");
}

fn lan_ips() -> Vec<String> {
    let mut ips = Vec::new();
    if let Ok(ip) = local_ip_address::local_ip() {
        ips.push(ip.to_string());
    }
    ips
}

/// `NAMIX_PIDFILE` 优先；生产包（旁路有 MANIFEST.json）默认写 `../app.pid`（即 dist/app.pid）。
fn write_pidfile() {
    let path = if let Ok(p) = std::env::var("NAMIX_PIDFILE") {
        std::path::PathBuf::from(p)
    } else if std::path::Path::new("MANIFEST.json").is_file() {
        std::path::PathBuf::from("../app.pid")
    } else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let pid = std::process::id();
    if let Err(err) = std::fs::write(&path, format!("{pid}\n")) {
        crate::log::warn!("pidfile {} write failed: {err}", path.display());
    } else {
        crate::log::info!("pidfile → {}", path.display());
    }
}
