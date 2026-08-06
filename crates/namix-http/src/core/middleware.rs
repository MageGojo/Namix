use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::request::Request;
use super::response::Response;
use super::routing::HandlerFn;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

pub type MiddlewareFn = Arc<dyn Fn(Request, Next) -> BoxFuture<Response> + Send + Sync + 'static>;

/// 中间件调用链上下一环。
pub struct Next {
    middlewares: Arc<Vec<MiddlewareFn>>,
    index: usize,
    handler: HandlerFn,
}

impl Next {
    pub(crate) fn new(
        middlewares: Arc<Vec<MiddlewareFn>>,
        index: usize,
        handler: HandlerFn,
    ) -> Self {
        Self {
            middlewares,
            index,
            handler,
        }
    }

    /// 继续往后执行（下一个中间件，或最终控制器）。
    pub async fn run(self, req: Request) -> Response {
        if let Some(mw) = self.middlewares.get(self.index).cloned() {
            let next = Next::new(self.middlewares, self.index + 1, self.handler);
            mw(req, next).await
        } else {
            (self.handler)(req).await
        }
    }
}

pub fn wrap_middleware<F, Fut>(f: F) -> MiddlewareFn
where
    F: Fn(Request, Next) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    let f = Arc::new(f);
    Arc::new(move |req, next| {
        let f = Arc::clone(&f);
        Box::pin(async move { f(req, next).await })
    })
}
