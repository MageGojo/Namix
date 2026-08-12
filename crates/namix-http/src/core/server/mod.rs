//! HTTP 服务启动。
//!
//! 底层：
//! - HTTP/1.1：hyper（明文 TCP）
//! - HTTP/2：hyper + rustls（HTTPS，ALPN `h2`）
//! - HTTP/3：quinn + h3（QUIC/UDP，ALPN `h3`）
//!
//! 支持 `SO_REUSEPORT` 双进程重叠监听（热更新）+ SIGTERM/Ctrl-C 优雅排水。

mod tls;

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request as HyperRequest, Response as HyperResponse};
use hyper_util::rt::{TokioExecutor, TokioIo};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_rustls::TlsAcceptor;

pub use tls::TlsConfig;

use super::middleware::{MiddlewareFn, Next, wrap_middleware};
use super::request::Request;
use super::response::{Body, Response};
use super::routing::{RouteCatalog, Router, WsHandshakeOutcome};

struct App {
    router: Router,
    middlewares: Arc<Vec<MiddlewareFn>>,
    routes: Arc<RouteCatalog>,
}

/// 框架服务器。业务侧组装 bind / tls / routes / middleware，然后 `run`。
pub struct Server {
    http_addr: Option<SocketAddr>,
    https_addr: Option<SocketAddr>,
    http3: bool,
    tls: Option<TlsConfig>,
    router: Router,
    middlewares: Vec<MiddlewareFn>,
}

impl Server {
    pub fn new() -> Self {
        Self {
            http_addr: Some(SocketAddr::from(([127, 0, 0, 1], 3000))),
            https_addr: None,
            http3: true,
            tls: None,
            router: Router::new(),
            middlewares: Vec::new(),
        }
    }

    /// 明文 HTTP/1.1（hyper）。
    pub fn bind(mut self, addr: impl AsRef<str>) -> Self {
        self.http_addr = Some(parse_addr(addr.as_ref()));
        self
    }

    /// 关闭明文 HTTP（只跑 HTTPS / HTTP/3）。
    pub fn disable_http(mut self) -> Self {
        self.http_addr = None;
        self
    }

    /// HTTPS：HTTP/1.1 + HTTP/2（需配合 [`Server::tls`] / [`Server::tls_self_signed`]）。
    pub fn https(mut self, addr: impl AsRef<str>) -> Self {
        self.https_addr = Some(parse_addr(addr.as_ref()));
        self
    }

    pub fn tls(mut self, tls: TlsConfig) -> Self {
        self.tls = Some(tls);
        self
    }

    pub fn tls_pem(
        self,
        cert_path: impl AsRef<std::path::Path>,
        key_path: impl AsRef<std::path::Path>,
    ) -> Self {
        self.tls(TlsConfig::from_pem_files(cert_path, key_path))
    }

    pub fn tls_self_signed(self, hostnames: &[&str]) -> Self {
        self.tls(TlsConfig::self_signed(hostnames))
    }

    /// 是否在 `https` 同一地址上启用 HTTP/3（UDP/QUIC）。默认 `true`。
    pub fn http3(mut self, enabled: bool) -> Self {
        self.http3 = enabled;
        self
    }

    /// 改端口（保留当前 IP；HTTP / HTTPS 各自改自己的端口）。
    pub fn port(mut self, port: u16) -> Self {
        if let Some(addr) = self.http_addr.as_mut() {
            addr.set_port(port);
        }
        self
    }

    /// HTTPS 端口（需已配置 https 监听）。
    pub fn https_port(mut self, port: u16) -> Self {
        if let Some(addr) = self.https_addr.as_mut() {
            addr.set_port(port);
        } else {
            let ip = self
                .http_addr
                .map(|a| a.ip())
                .unwrap_or_else(|| std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
            self.https_addr = Some(SocketAddr::new(ip, port));
        }
        self
    }

    /// `-h` / 局域网：`true` 绑定 `0.0.0.0`，手机等同网段可访问。
    pub fn lan(mut self, enabled: bool) -> Self {
        let ip = if enabled {
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        } else {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        };
        if let Some(addr) = self.http_addr.as_mut() {
            addr.set_ip(ip);
        }
        if let Some(addr) = self.https_addr.as_mut() {
            addr.set_ip(ip);
        }
        self
    }

    /// 本地一键 HTTPS（自签证书）。`https_port` 默认 `port + 443` 若未单独设置则用传入端口。
    pub fn local_https(mut self, enabled: bool, https_port: u16, hosts: &[&str]) -> Self {
        if !enabled {
            self.https_addr = None;
            self.tls = None;
            return self;
        }
        let ip = self
            .http_addr
            .map(|a| a.ip())
            .unwrap_or_else(|| std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        self.https_addr = Some(SocketAddr::new(ip, https_port));
        self.tls = Some(TlsConfig::self_signed(hosts));
        self
    }

    pub fn http_addr(&self) -> Option<SocketAddr> {
        self.http_addr
    }

    pub fn https_addr(&self) -> Option<SocketAddr> {
        self.https_addr
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

    pub async fn run(self) -> std::io::Result<()> {
        let routes = Arc::new(self.router.catalog());
        let app = Arc::new(App {
            router: self.router,
            middlewares: Arc::new(self.middlewares),
            routes,
        });

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let inflight = Arc::new(AtomicUsize::new(0));
        let mut set = tokio::task::JoinSet::new();

        if let Some(addr) = self.http_addr {
            // Bind before reporting ready. A rolling updater must verify the
            // exact candidate process, not an old peer sharing SO_REUSEPORT.
            let listener = bind_tcp(addr)?;
            let app = Arc::clone(&app);
            let rx = shutdown_rx.clone();
            let inflight = Arc::clone(&inflight);
            set.spawn(async move { serve_http1(listener, app, rx, inflight).await });
            println!("namix http/1.1  on http://{addr}");
        }

        if let Some(https_addr) = self.https_addr {
            let tls = self.tls.clone().unwrap_or_else(|| {
                panic!("已配置 .https(...)，但未设置 TLS：请调用 .tls_self_signed(&[...]) 或 .tls_pem(cert, key)");
            });

            let https_listener = bind_tcp(https_addr)?;
            let app_https = Arc::clone(&app);
            let tls_https = tls.clone();
            let rx = shutdown_rx.clone();
            let inflight_https = Arc::clone(&inflight);
            set.spawn(async move {
                serve_https(https_listener, tls_https, app_https, rx, inflight_https).await
            });
            println!("namix http/1.1+2 on https://{https_addr}");

            if self.http3 {
                let app_h3 = Arc::clone(&app);
                let rx = shutdown_rx.clone();
                set.spawn(async move { serve_http3(https_addr, tls, app_h3, rx).await });
                println!("namix http/3     on https://{https_addr} (QUIC/UDP)");
            }
        } else if self.http3 {
            println!("namix warning: http3=true 但未配置 .https(...)，已跳过 HTTP/3");
        }

        if set.is_empty() {
            panic!("没有可监听的端口：至少配置 .bind(...) 或 .https(...)");
        }
        write_readyfile();

        let signaled = match wait_for_signal_or_task(&mut set).await {
            Ok(signaled) => signaled,
            Err(err) => {
                eprintln!("namix: listener failed: {err}");
                let _ = shutdown_tx.send(true);
                join_listeners(&mut set).await;
                clear_pidfile();
                return Err(err);
            }
        };
        println!(
            "namix: graceful shutdown… ({})",
            if signaled { "signal" } else { "listener exit" }
        );
        let _ = shutdown_tx.send(true);
        join_listeners(&mut set).await;
        drain_inflight(&inflight, Duration::from_secs(15)).await;
        clear_pidfile();
        println!("namix: stopped");
        Ok(())
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

async fn join_listeners(set: &mut tokio::task::JoinSet<std::io::Result<()>>) {
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(Err(err)) => eprintln!("namix: listener exit: {err}"),
            Err(err) => eprintln!("namix: listener join error: {err}"),
            Ok(Ok(())) => {}
        }
    }
}

/// `true` = 收到停机信号；`false` = 某个 listener 正常结束。
async fn wait_for_signal_or_task(
    set: &mut tokio::task::JoinSet<std::io::Result<()>>,
) -> std::io::Result<bool> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sighup = signal(SignalKind::hangup())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => Ok(true),
            _ = sigterm.recv() => Ok(true),
            _ = sighup.recv() => Ok(true),
            joined = set.join_next() => listener_completion(joined),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => Ok(true),
            joined = set.join_next() => listener_completion(joined),
        }
    }
}

fn listener_completion(
    joined: Option<Result<std::io::Result<()>, tokio::task::JoinError>>,
) -> std::io::Result<bool> {
    match joined {
        None | Some(Ok(Ok(()))) => Ok(false),
        Some(Ok(Err(err))) => Err(err),
        Some(Err(err)) => Err(std::io::Error::other(format!(
            "listener task failed: {err}"
        ))),
    }
}

async fn drain_inflight(inflight: &AtomicUsize, timeout: Duration) {
    let start = std::time::Instant::now();
    while inflight.load(Ordering::SeqCst) > 0 && start.elapsed() < timeout {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let left = inflight.load(Ordering::SeqCst);
    if left > 0 {
        eprintln!("namix: drain timeout, {left} request(s) still in flight");
    }
}

fn clear_pidfile() {
    let path = if let Ok(p) = std::env::var("NAMIX_PIDFILE") {
        std::path::PathBuf::from(p)
    } else if std::path::Path::new("MANIFEST.json").is_file() {
        std::path::PathBuf::from("../app.pid")
    } else {
        return;
    };
    // A new rolling-release process may have replaced this shared pidfile.
    // Only remove it when it still points at this process.
    let ours = std::process::id().to_string();
    if std::fs::read_to_string(&path)
        .ok()
        .is_some_and(|value| value.trim() == ours)
    {
        let _ = std::fs::remove_file(path);
    }
}

fn write_readyfile() {
    let Ok(path) = std::env::var("NAMIX_READYFILE") else {
        return;
    };
    if let Err(error) = std::fs::write(&path, format!("{}\n", std::process::id())) {
        eprintln!("namix: write ready file {path} failed: {error}");
    }
}

fn parse_addr(addr: &str) -> SocketAddr {
    addr.parse()
        .unwrap_or_else(|e| panic!("无效监听地址 {addr}: {e}"))
}

/// TCP bind：`SO_REUSEADDR` + Unix `SO_REUSEPORT`，便于新旧进程重叠接流量。
fn bind_tcp(addr: SocketAddr) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};

    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    TcpListener::from_std(std::net::TcpListener::from(socket))
}

async fn serve_http1(
    listener: TcpListener,
    app: Arc<App>,
    mut shutdown: watch::Receiver<bool>,
    inflight: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let app = Arc::clone(&app);
                let inflight = Arc::clone(&inflight);
                inflight.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let service = service_fn(move |req| {
                        let app = Arc::clone(&app);
                        async move { handle_hyper(req, app, peer).await }
                    });
                    if let Err(err) = hyper::server::conn::http1::Builder::new()
                        .serve_connection(io, service)
                        .with_upgrades()
                        .await
                    {
                        eprintln!("http/1.1 connection error: {err}");
                    }
                    inflight.fetch_sub(1, Ordering::SeqCst);
                });
            }
        }
    }
    Ok(())
}

async fn serve_https(
    listener: TcpListener,
    tls: TlsConfig,
    app: Arc<App>,
    mut shutdown: watch::Receiver<bool>,
    inflight: Arc<AtomicUsize>,
) -> std::io::Result<()> {
    let acceptor = TlsAcceptor::from(tls.rustls_https());

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (stream, peer) = accepted?;
                let acceptor = acceptor.clone();
                let app = Arc::clone(&app);
                let inflight = Arc::clone(&inflight);
                inflight.fetch_add(1, Ordering::SeqCst);

                tokio::spawn(async move {
                    let tls_stream = match acceptor.accept(stream).await {
                        Ok(s) => s,
                        Err(err) => {
                            eprintln!("tls handshake error: {err}");
                            inflight.fetch_sub(1, Ordering::SeqCst);
                            return;
                        }
                    };

                    let io = TokioIo::new(tls_stream);
                    let service = service_fn(move |req| {
                        let app = Arc::clone(&app);
                        async move { handle_hyper(req, app, peer).await }
                    });

                    if let Err(err) = hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(io, service)
                        .await
                    {
                        eprintln!("https connection error: {err}");
                    }
                    inflight.fetch_sub(1, Ordering::SeqCst);
                });
            }
        }
    }
    Ok(())
}

async fn serve_http3(
    addr: SocketAddr,
    tls: TlsConfig,
    app: Arc<App>,
    mut shutdown: watch::Receiver<bool>,
) -> std::io::Result<()> {
    use h3_quinn::quinn::crypto::rustls::QuicServerConfig;

    let quic_tls = tls.rustls_http3();
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(quic_tls)
            .map_err(|e| std::io::Error::other(format!("quic tls config error: {e}")))?,
    ));
    let endpoint = quinn::Endpoint::server(server_config, addr)?;

    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    endpoint.close(0u32.into(), b"shutdown");
                    break;
                }
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let app = Arc::clone(&app);
                tokio::spawn(async move {
                    let conn = match incoming.await {
                        Ok(conn) => conn,
                        Err(err) => {
                            eprintln!("http/3 accept error: {err}");
                            return;
                        }
                    };

                    let peer = conn.remote_address();
                    let mut h3_conn = match h3::server::Connection::new(h3_quinn::Connection::new(conn)).await
                    {
                        Ok(c) => c,
                        Err(err) => {
                            eprintln!("http/3 connection error: {err}");
                            return;
                        }
                    };

                    loop {
                        match h3_conn.accept().await {
                            Ok(Some(resolver)) => {
                                let app = Arc::clone(&app);
                                tokio::spawn(async move {
                                    if let Err(err) = handle_h3(resolver, app, peer).await {
                                        eprintln!("http/3 request error: {err}");
                                    }
                                });
                            }
                            Ok(None) => break,
                            Err(err) => {
                                eprintln!("http/3 accept stream error: {err}");
                                break;
                            }
                        }
                    }
                });
            }
        }
    }

    Ok(())
}

async fn handle_hyper(
    req: HyperRequest<Incoming>,
    app: Arc<App>,
    peer: SocketAddr,
) -> Result<HyperResponse<Body>, Infallible> {
    // WebSocket：在消费 body 前保留 Hyper Upgrade，但先让普通 Namix
    // middleware 对轻量 Request 完成鉴权、限流与上下文注入。
    if crate::core::ws::is_upgrade_request(req.headers()) {
        let key = req
            .headers()
            .get("sec-websocket-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let mut namix_req = crate::core::ws::namix_request_from_hyper(&req, Vec::new());
        namix_req.set_routes(Arc::clone(&app.routes));
        namix_req.set_client_ip(peer.ip());

        match app
            .router
            .dispatch_ws_handshake(namix_req, Arc::clone(&app.middlewares))
            .await
        {
            WsHandshakeOutcome::Accepted {
                request,
                handler,
                middleware_response,
            } => {
                let response = crate::core::ws::switching_protocols_with_headers(
                    &key,
                    middleware_response.headers(),
                );
                tokio::spawn(async move {
                    crate::core::ws::run_upgraded(req, request, handler).await;
                });
                return Ok(response);
            }
            WsHandshakeOutcome::Rejected(response) => return Ok(response.into_inner()),
        }
    }

    let (parts, body) = req.into_parts();
    let collected = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            eprintln!("read body failed: {err}");
            Bytes::new()
        }
    };

    let mut request = Request::new(parts.method, parts.uri, parts.headers, collected);
    request.set_routes(Arc::clone(&app.routes));
    request.set_client_ip(peer.ip());
    let response = app
        .router
        .dispatch(request, Arc::clone(&app.middlewares))
        .await;
    Ok(response.into_inner())
}

async fn handle_h3<C>(
    resolver: h3::server::RequestResolver<C, Bytes>,
    app: Arc<App>,
    peer: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    C: h3::quic::Connection<Bytes>,
{
    let (req, mut stream) = resolver.resolve_request().await?;
    let (parts, _body) = req.into_parts();

    // h3 请求体
    let mut collected = Vec::new();
    while let Some(mut chunk) = stream.recv_data().await? {
        use bytes::Buf;
        while chunk.has_remaining() {
            let n = chunk.chunk().len();
            collected.extend_from_slice(chunk.chunk());
            chunk.advance(n);
        }
    }

    let mut request = Request::new(
        parts.method,
        parts.uri,
        parts.headers,
        Bytes::from(collected),
    );
    request.set_routes(Arc::clone(&app.routes));
    request.set_client_ip(peer.ip());
    let response = app
        .router
        .dispatch(request, Arc::clone(&app.middlewares))
        .await;

    let (status, headers, body) = response.into_status_headers_body().await;
    let mut builder = http::Response::builder().status(status);
    for (name, value) in headers.iter() {
        builder = builder.header(name, value);
    }
    let h3_response = builder.body(())?;
    stream.send_response(h3_response).await?;
    if !body.is_empty() {
        stream.send_data(body).await?;
    }
    stream.finish().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::listener_completion;

    #[test]
    fn listener_error_is_not_silently_treated_as_shutdown() {
        let result = listener_completion(Some(Ok(Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "port already in use",
        )))));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AddrInUse);
    }
}
