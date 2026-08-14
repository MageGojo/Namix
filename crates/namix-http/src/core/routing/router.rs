use std::future::Future;
use std::sync::{Arc, Mutex};

use http::Method;

use crate::core::content_type::ContentType;
use crate::core::error::AppError;
use crate::core::error_pages::{ErrorPage, ErrorPages};
use crate::core::handler::Handler;
use crate::core::middleware::{BoxFuture, MiddlewareFn, Next, wrap_middleware};
use crate::core::request::Request;
use crate::core::response::Response;

use crate::core::ws::{WsHandlerFn, WsRouteEntry};

use super::HandlerFn;
use super::catalog::RouteCatalog;
use super::path::PathPattern;
use super::route::Route;

type WsMatch = (Arc<WsRouteEntry>, Vec<(String, String)>);

pub(crate) enum WsHandshakeOutcome {
    Accepted {
        request: Box<Request>,
        handler: WsHandlerFn,
        middleware_response: Response,
    },
    Rejected(Response),
}

#[derive(Clone)]
struct WsMiddlewarePassed;

pub(crate) struct RouteEntry {
    pub method: Method,
    pub pattern: PathPattern,
    pub handler: HandlerFn,
    pub middlewares: Vec<MiddlewareFn>,
    pub name: Option<String>,
}

/// 路由表（框架能力；业务只负责填写路径与处理器）。
#[derive(Default, Clone)]
pub struct Router {
    routes: Vec<Arc<RouteEntry>>,
    ws_routes: Vec<Arc<WsRouteEntry>>,
    error_pages: ErrorPages,
}

impl Router {
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            ws_routes: Vec::new(),
            error_pages: ErrorPages::new(),
        }
    }

    pub(crate) fn from_entries(entries: Vec<RouteEntry>) -> Self {
        Self {
            routes: entries.into_iter().map(Arc::new).collect(),
            ws_routes: Vec::new(),
            error_pages: ErrorPages::new(),
        }
    }

    pub(crate) fn from_ws_entries(entries: Vec<WsRouteEntry>) -> Self {
        Self {
            routes: Vec::new(),
            ws_routes: entries.into_iter().map(Arc::new).collect(),
            error_pages: ErrorPages::new(),
        }
    }

    pub fn merge(mut self, other: Router) -> Self {
        self.routes.extend(other.routes);
        self.ws_routes.extend(other.ws_routes);
        self.error_pages = self.error_pages.merge(other.error_pages);
        self
    }

    /// 把另一份错误页表垫在下面：当前 router（通常是 `web.rs`）覆盖相同状态码。
    pub fn merge_error_pages(mut self, pages: ErrorPages) -> Self {
        self.error_pages = pages.merge(self.error_pages);
        self
    }

    /// 给已有路由统一加前缀。
    pub fn nest(self, prefix: &str, other: Router) -> Self {
        self.nest_with_middleware(prefix, other, Vec::new())
    }

    /// 分组注册：前缀 + 闭包内路由/中间件。
    ///
    /// ```ignore
    /// Router::new().group("/users", |r| {
    ///     r.get("/", index).get("/:id", show).middleware(require_admin)
    /// })
    /// ```
    pub fn group<F>(self, prefix: &str, build: F) -> Self
    where
        F: FnOnce(Router) -> Router,
    {
        self.nest(prefix, build(Router::new()))
    }

    pub fn nest_with_middleware(
        mut self,
        prefix: &str,
        other: Router,
        group_middlewares: Vec<MiddlewareFn>,
    ) -> Self {
        for entry in other.routes {
            let path = PathPattern::join(prefix, &entry.pattern.raw);
            let mut middlewares = group_middlewares.clone();
            middlewares.extend(entry.middlewares.iter().cloned());
            self.routes.push(Arc::new(RouteEntry {
                method: entry.method.clone(),
                pattern: PathPattern::parse(&path),
                handler: Arc::clone(&entry.handler),
                middlewares,
                name: entry.name.clone(),
            }));
        }
        for entry in other.ws_routes {
            let path = PathPattern::join(prefix, &entry.pattern.raw);
            let mut middlewares = group_middlewares.clone();
            middlewares.extend(entry.middlewares.iter().cloned());
            self.ws_routes.push(Arc::new(WsRouteEntry {
                pattern: PathPattern::parse(&path),
                handler: Arc::clone(&entry.handler),
                middlewares,
                name: entry.name.clone(),
            }));
        }
        self.error_pages = self.error_pages.merge(other.error_pages);
        self
    }

    pub fn get<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        self.merge(Route::get(path, handler).register())
    }

    pub fn post<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        self.merge(Route::post(path, handler).register())
    }

    pub fn put<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        self.merge(Route::put(path, handler).register())
    }

    pub fn patch<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        self.merge(Route::patch(path, handler).register())
    }

    pub fn delete<H, T>(self, path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        self.merge(Route::delete(path, handler).register())
    }

    pub fn route<H, T>(self, method: Method, path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        self.merge(Route::new(method, path, handler).register())
    }

    /// 可选：某一个 HTTP 状态的 HTML 错误页。不注册则保持框架默认。
    ///
    /// 渲染器返回的状态码会被强制成 `status`，避免写成 200。
    pub fn error_page<F>(mut self, status: u16, render: F) -> Self
    where
        F: Fn(&Request, ErrorPage) -> Response + Send + Sync + 'static,
    {
        self.error_pages = std::mem::take(&mut self.error_pages).page(status, render);
        self
    }

    /// 可选：其余 HTML 错误共用一页。具体状态（[`error_page`](Self::error_page)）优先。
    pub fn error_pages<F>(mut self, render: F) -> Self
    where
        F: Fn(&Request, ErrorPage) -> Response + Send + Sync + 'static,
    {
        self.error_pages = std::mem::take(&mut self.error_pages).any(render);
        self
    }

    /// 按名称查找路由模式（含 `:param`）。
    pub fn route_path(&self, name: &str) -> Option<&str> {
        self.routes
            .iter()
            .find(|r| r.name.as_deref() == Some(name))
            .map(|r| r.pattern.raw.as_str())
    }

    /// 导出命名路由表，供 `req.url` / `redirect_route` / 前端 JSON 使用。
    pub fn catalog(&self) -> RouteCatalog {
        let mut catalog = RouteCatalog::new();
        for route in &self.routes {
            if let Some(name) = &route.name {
                catalog.insert_with_method(
                    name.clone(),
                    route.pattern.raw.clone(),
                    Some(route.method.as_str()),
                );
            }
        }
        for route in &self.ws_routes {
            if let Some(name) = &route.name {
                // 握手是 GET；标注 WS 便于前端区分
                catalog.insert_with_method(name.clone(), route.pattern.raw.clone(), Some("WS"));
            }
        }
        catalog
    }

    /// 匹配 WebSocket 路由（仅 path）。
    pub(crate) fn match_ws(&self, path: &str) -> Option<WsMatch> {
        self.ws_routes.iter().find_map(|route| {
            let params = route.pattern.match_path_ordered(path)?;
            Some((Arc::clone(route), params))
        })
    }

    /// Run a WebSocket HTTP handshake through the same global and route
    /// middleware model as regular routes. The terminal handler captures the
    /// fully mutated request and marks its response with a private extension;
    /// a short-circuit can therefore never be confused with permission to
    /// upgrade merely by returning status 101.
    pub(crate) async fn dispatch_ws_handshake(
        &self,
        mut req: Request,
        global_middlewares: Arc<Vec<MiddlewareFn>>,
    ) -> WsHandshakeOutcome {
        let Some((route, params)) = self.match_ws(req.path()) else {
            let handler: HandlerFn = Arc::new(|_req| {
                Box::pin(async {
                    crate::core::ws::reject(
                        http::StatusCode::NOT_FOUND,
                        "websocket route not found",
                    )
                }) as BoxFuture<Response>
            });
            let response = Next::new(global_middlewares, 0, handler).run(req).await;
            return WsHandshakeOutcome::Rejected(sanitize_ws_rejection(response));
        };

        req.set_params(params);
        let mut chain = Vec::with_capacity(global_middlewares.len() + route.middlewares.len());
        chain.extend(global_middlewares.iter().cloned());
        chain.extend(route.middlewares.iter().cloned());

        let captured = Arc::new(Mutex::new(None));
        let terminal_capture = Arc::clone(&captured);
        let terminal: HandlerFn = Arc::new(move |request| {
            let terminal_capture = Arc::clone(&terminal_capture);
            Box::pin(async move {
                let mut response = Response::new(
                    http::StatusCode::SWITCHING_PROTOCOLS,
                    ContentType::Text,
                    bytes::Bytes::new(),
                );
                if let Ok(mut slot) = terminal_capture.lock() {
                    *slot = Some(request);
                    response.insert_extension(WsMiddlewarePassed);
                }
                response
            })
        });

        let response = Next::new(Arc::new(chain), 0, terminal).run(req).await;
        let passed = response.status() == http::StatusCode::SWITCHING_PROTOCOLS
            && response.extension::<WsMiddlewarePassed>().is_some();
        let request = if passed {
            captured.lock().ok().and_then(|mut slot| slot.take())
        } else {
            None
        };

        match request {
            Some(request) => WsHandshakeOutcome::Accepted {
                request: Box::new(request),
                handler: Arc::clone(&route.handler),
                middleware_response: response,
            },
            None => WsHandshakeOutcome::Rejected(sanitize_ws_rejection(response)),
        }
    }

    fn push_middleware_fn(&mut self, mw: MiddlewareFn) {
        for entry in &mut self.routes {
            let mut next = Vec::with_capacity(entry.middlewares.len() + 1);
            next.push(Arc::clone(&mw));
            next.extend(entry.middlewares.iter().cloned());
            *entry = Arc::new(RouteEntry {
                method: entry.method.clone(),
                pattern: entry.pattern.clone(),
                handler: Arc::clone(&entry.handler),
                middlewares: next,
                name: entry.name.clone(),
            });
        }
        for entry in &mut self.ws_routes {
            let mut next = Vec::with_capacity(entry.middlewares.len() + 1);
            next.push(Arc::clone(&mw));
            next.extend(entry.middlewares.iter().cloned());
            *entry = Arc::new(WsRouteEntry {
                pattern: entry.pattern.clone(),
                handler: Arc::clone(&entry.handler),
                middlewares: next,
                name: entry.name.clone(),
            });
        }
    }

    /// 给当前 router 内所有路由前置中间件（组中间件）。
    ///
    /// 推荐用 [`middleware`](Self::middleware)；`layer` 为同义别名。
    pub fn middleware<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.push_middleware_fn(wrap_middleware(f));
        self
    }

    /// [`middleware`](Self::middleware) 的别名（兼容旧代码）。
    pub fn layer<F, Fut>(self, f: F) -> Self
    where
        F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.middleware(f)
    }

    pub(crate) async fn dispatch(
        &self,
        mut req: Request,
        global_middlewares: Arc<Vec<MiddlewareFn>>,
    ) -> Response {
        req.set(self.error_pages.clone());

        let method = req.method().clone();
        let path = req.path().to_string();

        let matched = self.routes.iter().find_map(|route| {
            if route.method != method {
                return None;
            }
            let params = route.pattern.match_path_ordered(&path)?;
            Some((Arc::clone(route), params))
        });

        let Some((route, params)) = matched else {
            let handler: HandlerFn = Arc::new(|req| {
                Box::pin(async move { unmatched_response(req) }) as BoxFuture<Response>
            });
            return Next::new(global_middlewares, 0, handler).run(req).await;
        };

        req.set_params(params);

        let mut chain = Vec::with_capacity(global_middlewares.len() + route.middlewares.len());
        chain.extend(global_middlewares.iter().cloned());
        chain.extend(route.middlewares.iter().cloned());

        Next::new(Arc::new(chain), 0, Arc::clone(&route.handler))
            .run(req)
            .await
    }
}

fn unmatched_response(req: Request) -> Response {
    AppError::NotFound.into_response_for(&req)
}

fn sanitize_ws_rejection(mut response: Response) -> Response {
    if response.status() == http::StatusCode::SWITCHING_PROTOCOLS {
        response.set_status(http::StatusCode::INTERNAL_SERVER_ERROR);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::controller::text;
    use crate::core::middleware::{Next, wrap_middleware};
    use crate::core::proxy::TrustedProxies;
    use crate::core::request::Request;
    use crate::core::response::Response;
    use crate::core::ws::{WsSocket, switching_protocols_with_headers};
    use http::{Method, Request as HttpRequest};

    async fn ok(_req: Request) -> Response {
        text("ok")
    }

    async fn block(req: Request, _next: Next) -> Response {
        text(format!("blocked:{}", req.header_or("x", "-")))
    }

    async fn mark(mut req: Request, next: Next) -> Response {
        req.set_attr("a", "1");
        next.run(req).await.with_header("x-m", "yes")
    }

    async fn ws_handler(_req: Request, _socket: WsSocket) {}

    async fn global_block(_req: Request, _next: Next) -> Response {
        Response::new(
            http::StatusCode::UNAUTHORIZED,
            ContentType::Text,
            "global blocked",
        )
        .with_header("x-blocked-by", "global")
    }

    async fn route_block(_req: Request, _next: Next) -> Response {
        Response::new(
            http::StatusCode::FORBIDDEN,
            ContentType::Text,
            "route blocked",
        )
        .with_header("x-blocked-by", "route")
    }

    async fn request_id(mut req: Request, next: Next) -> Response {
        req.set_attr("test.request_id", "req-42");
        next.run(req).await.with_header("x-request-id", "req-42")
    }

    async fn mark_ws_route(mut req: Request, next: Next) -> Response {
        let inherited = req.attr_or("test.request_id", "missing").to_string();
        req.set_attr("test.route_saw", inherited);
        next.run(req)
            .await
            .with_header("strict-transport-security", "max-age=31536000")
    }

    async fn forge_upgrade_after_next(req: Request, next: Next) -> Response {
        let _ = next.run(req).await;
        Response::new(
            http::StatusCode::SWITCHING_PROTOCOLS,
            ContentType::Text,
            "forged",
        )
    }

    fn req(path: &str) -> Request {
        let http_req: HttpRequest<()> = HttpRequest::builder()
            .method(Method::GET)
            .uri(path)
            .body(())
            .unwrap();
        let (parts, _) = http_req.into_parts();
        Request::new(parts.method, parts.uri, parts.headers, bytes::Bytes::new())
    }

    fn ws_req(path: &str) -> Request {
        let mut request = req(path);
        request.set_header("host", "app.test");
        request.set_header("origin", "https://app.test");
        request.set_header("connection", "Upgrade");
        request.set_header("upgrade", "websocket");
        request.set_header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==");
        request.set_header("x-forwarded-for", "198.51.100.20");
        request.set_client_ip("127.0.0.1".parse().unwrap());
        request
    }

    #[tokio::test]
    async fn middleware_runs() {
        let router = Router::new().get("/x", ok).middleware(block);
        let resp = router
            .dispatch(req("/x"), std::sync::Arc::new(vec![]))
            .await;
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert!(resp.header("content-type").is_some());
        // body should be blocked not ok
        let (_s, _h, body) = resp.into_status_headers_body().await;
        assert_eq!(&body[..], b"blocked:-");
    }

    #[tokio::test]
    async fn attr_and_response_header() {
        async fn h(req: Request) -> Response {
            text(format!("a={}", req.attr_or("a", "-")))
        }
        let router = Router::new().get("/x", h).middleware(mark);
        let resp = router
            .dispatch(req("/x"), std::sync::Arc::new(vec![]))
            .await;
        assert_eq!(resp.header("x-m"), Some("yes"));
        let (_s, _h, body) = resp.into_status_headers_body().await;
        assert_eq!(&body[..], b"a=1");
    }

    #[tokio::test]
    async fn websocket_global_and_route_middleware_short_circuit_unchanged() {
        let router = Route::ws("/ws", ws_handler).register();
        let outcome = router
            .dispatch_ws_handshake(ws_req("/ws"), Arc::new(vec![wrap_middleware(global_block)]))
            .await;
        let WsHandshakeOutcome::Rejected(response) = outcome else {
            panic!("global middleware unexpectedly allowed upgrade");
        };
        assert_eq!(response.status(), http::StatusCode::UNAUTHORIZED);
        assert_eq!(response.header("x-blocked-by"), Some("global"));
        assert_eq!(
            response.into_status_headers_body().await.2,
            "global blocked"
        );

        let router = Route::ws("/ws", ws_handler)
            .middleware(route_block)
            .register();
        let outcome = router
            .dispatch_ws_handshake(ws_req("/ws"), Arc::new(Vec::new()))
            .await;
        let WsHandshakeOutcome::Rejected(response) = outcome else {
            panic!("route middleware unexpectedly allowed upgrade");
        };
        assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
        assert_eq!(response.header("x-blocked-by"), Some("route"));
        assert_eq!(response.into_status_headers_body().await.2, "route blocked");
    }

    #[tokio::test]
    async fn websocket_middleware_mutations_and_security_headers_reach_upgrade() {
        let router = Route::ws("/ws/:room", ws_handler)
            .middleware(mark_ws_route)
            .register();
        let globals = vec![
            TrustedProxies::new(["127.0.0.1"]).unwrap().middleware(),
            wrap_middleware(request_id),
        ];

        let outcome = router
            .dispatch_ws_handshake(ws_req("/ws/general"), Arc::new(globals))
            .await;
        let WsHandshakeOutcome::Accepted {
            request,
            middleware_response,
            ..
        } = outcome
        else {
            panic!("middleware unexpectedly rejected upgrade");
        };

        assert_eq!(request.param("room"), Some("general"));
        assert_eq!(request.attr("test.request_id"), Some("req-42"));
        assert_eq!(request.attr("test.route_saw"), Some("req-42"));
        assert_eq!(request.client_ip(), Some("198.51.100.20".parse().unwrap()));
        assert_eq!(middleware_response.header("x-request-id"), Some("req-42"));

        let response = switching_protocols_with_headers(
            "dGhlIHNhbXBsZSBub25jZQ==",
            middleware_response.headers(),
        );
        assert_eq!(response.status(), http::StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(response.headers()["x-request-id"], "req-42");
        assert_eq!(
            response.headers()["strict-transport-security"],
            "max-age=31536000"
        );
        assert_eq!(
            response.headers()["sec-websocket-accept"],
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        assert!(response.headers().get("content-type").is_none());
    }

    #[tokio::test]
    async fn websocket_requires_the_private_terminal_marker() {
        let router = Route::ws("/ws", ws_handler)
            .middleware(forge_upgrade_after_next)
            .register();
        let outcome = router
            .dispatch_ws_handshake(ws_req("/ws"), Arc::new(Vec::new()))
            .await;
        let WsHandshakeOutcome::Rejected(response) = outcome else {
            panic!("replacement response unexpectedly allowed upgrade");
        };
        assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn router_group_middleware_applies_to_websocket_routes() {
        let router = Router::new().group("/api", |router| {
            router
                .merge(Route::ws("/events", ws_handler).register())
                .middleware(route_block)
        });
        let outcome = router
            .dispatch_ws_handshake(ws_req("/api/events"), Arc::new(Vec::new()))
            .await;
        let WsHandshakeOutcome::Rejected(response) = outcome else {
            panic!("group middleware unexpectedly allowed upgrade");
        };
        assert_eq!(response.status(), http::StatusCode::FORBIDDEN);
    }
}
