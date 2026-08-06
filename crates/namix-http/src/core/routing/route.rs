use http::Method;

use crate::core::handler::{Handler, into_handler_fn};
use crate::core::middleware::{MiddlewareFn, Next, wrap_middleware};
use crate::core::request::Request;
use crate::core::response::Response;
use crate::core::ws::{IntoWsHandler, WsHandlerFn, WsRouteEntry};

use super::HandlerFn;
use super::path::PathPattern;
use super::router::{RouteEntry, Router};

use std::future::Future;

/// 单条路由的链式构建器：路径 → 处理器 → 中间件 → 命名 → 注册。
pub struct Route {
    method: Method,
    pattern: PathPattern,
    handler: HandlerFn,
    middlewares: Vec<MiddlewareFn>,
    name: Option<String>,
}

/// WebSocket 路由构建器：`Route::ws("/ws/echo", handler).name("ws.echo").register()`
pub struct WsRoute {
    pattern: PathPattern,
    handler: WsHandlerFn,
    name: Option<String>,
}

impl Route {
    pub fn get<H, T>(path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        Self::new(Method::GET, path, handler)
    }

    pub fn post<H, T>(path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        Self::new(Method::POST, path, handler)
    }

    pub fn put<H, T>(path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        Self::new(Method::PUT, path, handler)
    }

    pub fn patch<H, T>(path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        Self::new(Method::PATCH, path, handler)
    }

    pub fn delete<H, T>(path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        Self::new(Method::DELETE, path, handler)
    }

    pub fn new<H, T>(method: Method, path: &str, handler: H) -> Self
    where
        H: Handler<T>,
    {
        Self {
            method,
            pattern: PathPattern::parse(path),
            handler: into_handler_fn(handler),
            middlewares: Vec::new(),
            name: None,
        }
    }

    pub fn middleware<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        self.middlewares.push(wrap_middleware(f));
        self
    }

    /// 给路由起一个**反向 URL 用的名字**（Laravel `->name('login')`）。
    ///
    /// - **可省略**：不写也能按 path 访问；名字只用于 `redirect` / `route::…` / 前端 `route.xxx()`。
    /// - 注册处推荐短字符串：`.name("login")` / `.name("login.submit")`。
    /// - 使用处用类型化名补全：`req.redirect_to(route::main::login)`。
    pub fn name(mut self, name: impl super::IntoRouteName) -> Self {
        self.name = Some(name.into_route_name());
        self
    }

    /// 注册为只含本路由的 [`Router`]，便于 `.merge(...)`。
    pub fn register(self) -> Router {
        Router::from_entries(vec![RouteEntry {
            method: self.method,
            pattern: self.pattern,
            handler: self.handler,
            middlewares: self.middlewares,
            name: self.name,
        }])
    }

    /// WebSocket 端点（`GET` + Upgrade）。WSS = 同一路径走 HTTPS 监听。
    ///
    /// 处理器签名：`async fn(socket)` 或 `async fn(req, socket)`。
    pub fn ws<H, T>(path: &str, handler: H) -> WsRoute
    where
        H: IntoWsHandler<T>,
    {
        WsRoute {
            pattern: PathPattern::parse(path),
            handler: handler.into_ws_handler(),
            name: None,
        }
    }
}

impl WsRoute {
    pub fn name(mut self, name: impl super::IntoRouteName) -> Self {
        self.name = Some(name.into_route_name());
        self
    }

    pub fn register(self) -> Router {
        Router::from_ws_entries(vec![WsRouteEntry {
            pattern: self.pattern,
            handler: self.handler,
            name: self.name,
        }])
    }
}
