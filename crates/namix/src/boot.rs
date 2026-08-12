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
use crate::config::NamixToml;
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

    /// Build-time contract export used by `nx build`; does not open listeners.
    #[arg(long = "export-routes", hide = true)]
    export_routes: bool,
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
        if args.export_routes {
            let catalog = framework_routes(self.router.clone()).catalog();
            write_routes_exports(&catalog);
            println!(
                "namix routes exported ({} named routes)",
                catalog.names().count()
            );
            return Ok(());
        }
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
        crate::config::install_session_lifetimes(&cfg.session);
        if let Some(secret) = crate::config::session_secret() {
            crate::crypt::Crypt::install(secret);
        } else if let Ok(dev) = std::env::var("NAMIX_SESSION_SECRET") {
            crate::crypt::Crypt::install(&dev);
        } else {
            // Development: derive an ephemeral key so flash seals still work.
            let ephemeral = format!("dev-crypt-{}", std::process::id());
            crate::crypt::Crypt::install(&ephemeral);
        }
        crate::crypt::install_http_cookie_crypt();
        let sessions = crate::session::store_from_config(&cfg.session)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        crate::log::info!(
            "session → driver={} shared={} lifetime={}s jwt={}s",
            cfg.session.driver,
            sessions.is_shared(),
            cfg.session.lifetime_secs,
            cfg.session.jwt_lifetime_secs
        );
        crate::session::install(sessions);

        crate::mail::try_init(&cfg.mail).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("mail initialization failed: {error}"),
            )
        })?;
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
                // Connection URLs may embed credentials. Keep them out of
                // stdout/tracing while still exposing the selected driver.
                crate::log::info!("database → driver={}", cfg.database.driver);
                let db = crate::db::connect(&url, models).await.map_err(|error| {
                    std::io::Error::other(format!("database connect failed: {error}"))
                })?;
                if cfg.database.push_schema {
                    match db.push_schema().await {
                        Ok(()) => crate::log::info!("database schema pushed"),
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("already exists") {
                                crate::log::info!("database schema already present");
                            } else {
                                return Err(std::io::Error::other(format!(
                                    "push_schema failed: {e}"
                                )));
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

        // Outermost layer: even redirects/rejections emitted by later
        // middleware carry the same request id and duration span.
        server = attach_middleware(server, crate::request_id_middleware());

        if !cfg.security.trusted_proxies.is_empty() {
            let trusted = crate::TrustedProxies::new(&cfg.security.trusted_proxies)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
            server = attach_middleware(server, trusted.middleware());
        }

        if cfg.security.csrf {
            let csrf = CsrfConfig {
                trusted_origins: cfg.security.csrf_trusted_origins.clone(),
                // Development may expose HTTP and self-signed HTTPS together;
                // a Secure token issued over HTTP would never be echoed by the
                // browser. Production is HTTPS (directly or at the trusted edge).
                secure_cookie: cfg.is_production(),
                ..CsrfConfig::default()
            };
            server = attach_middleware(server, crate::CsrfProtection::new(csrf).middleware());
        }
        // Application hydration/auth middleware runs after request identity,
        // trusted-proxy resolution and browser mutation protection, but before
        // user-scoped rate limiting.
        for mw in self.middlewares {
            server = attach_middleware(server, mw);
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
        // #[server] → 单次 POST /api/a（公钥嵌入 WASM；action_seal 可关）
        crate::server_fn::configure(&self.app, cfg.features.action_seal);
        crate::log::info!(
            "action_seal = {} (NAMIX_ACTION_SEAL overrides toml)",
            crate::server_fn::action_seal_enabled()
        );
        let router = framework_routes(self.router);

        let catalog = router.catalog();
        write_routes_exports(&catalog);
        server = server.routes(router);

        print_access_urls(&server, args.lan || app_cfg.lan);
        let _pidfile = install_pidfile();
        server.run().await
    }
}

fn framework_routes(router: Router) -> Router {
    let router = router
        .merge(
            Route::get("/__namix/health", namix_health)
                .name("__namix.health")
                .register(),
        )
        .merge(
            Route::get("/__namix/routes", namix_routes_json)
                .name("__namix.routes")
                .register(),
        )
        .merge(crate::server_fn::routes());

    #[cfg(feature = "pages")]
    let router = router.merge(crate::pages::routes());

    router
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
    println!("--------- namix starting --------");
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
struct PidfileGuard {
    path: std::path::PathBuf,
    pid: String,
}

impl Drop for PidfileGuard {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path)
            .ok()
            .is_some_and(|value| value.trim() == self.pid)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn install_pidfile() -> Option<PidfileGuard> {
    let path = if let Ok(p) = std::env::var("NAMIX_PIDFILE") {
        std::path::PathBuf::from(p)
    } else if std::path::Path::new("MANIFEST.json").is_file() {
        std::path::PathBuf::from("../app.pid")
    } else {
        return None;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let pid = std::process::id();
    if let Err(err) = std::fs::write(&path, format!("{pid}\n")) {
        crate::log::warn!("pidfile {} write failed: {err}", path.display());
        None
    } else {
        crate::log::info!("pidfile → {}", path.display());
        Some(PidfileGuard {
            path,
            pid: pid.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::PidfileGuard;

    #[test]
    fn pidfile_guard_never_removes_a_newer_process_pid() {
        let path = std::env::temp_dir().join(format!(
            "namix-pid-guard-{}-{}.pid",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::write(&path, "22\n").unwrap();
        drop(PidfileGuard {
            path: path.clone(),
            pid: "11".into(),
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "22\n");
        drop(PidfileGuard {
            path: path.clone(),
            pid: "22".into(),
        });
        assert!(!path.exists());
    }
}
