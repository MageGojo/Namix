use std::future::Future;
use std::sync::Arc;

use http::Method;

use crate::core::controller;
use crate::core::handler::Handler;
use crate::core::middleware::{BoxFuture, MiddlewareFn, Next, wrap_middleware};
use crate::core::request::Request;
use crate::core::response::Response;

use crate::core::ws::WsRouteEntry;

use super::HandlerFn;
use super::catalog::RouteCatalog;
use super::path::PathPattern;
use super::route::Route;

type WsMatch = (Arc<WsRouteEntry>, Vec<(String, String)>);

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
}

impl Router {
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            ws_routes: Vec::new(),
        }
    }

    pub(crate) fn from_entries(entries: Vec<RouteEntry>) -> Self {
        Self {
            routes: entries.into_iter().map(Arc::new).collect(),
            ws_routes: Vec::new(),
        }
    }

    pub(crate) fn from_ws_entries(entries: Vec<WsRouteEntry>) -> Self {
        Self {
            routes: Vec::new(),
            ws_routes: entries.into_iter().map(Arc::new).collect(),
        }
    }

    pub fn merge(mut self, other: Router) -> Self {
        self.routes.extend(other.routes);
        self.ws_routes.extend(other.ws_routes);
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
            self.ws_routes.push(Arc::new(WsRouteEntry {
                pattern: PathPattern::parse(&path),
                handler: Arc::clone(&entry.handler),
                name: entry.name.clone(),
            }));
        }
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
            let handler: HandlerFn =
                Arc::new(|_req| Box::pin(async { controller::not_found() }) as BoxFuture<Response>);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::controller::text;
    use crate::core::middleware::Next;
    use crate::core::request::Request;
    use crate::core::response::Response;
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

    fn req(path: &str) -> Request {
        let http_req: HttpRequest<()> = HttpRequest::builder()
            .method(Method::GET)
            .uri(path)
            .body(())
            .unwrap();
        let (parts, _) = http_req.into_parts();
        Request::new(parts.method, parts.uri, parts.headers, bytes::Bytes::new())
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
}
