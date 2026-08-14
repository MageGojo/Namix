//! 把「带提取器的异步函数」和「同步闭包」收成统一的 HandlerFn。

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use super::extract::FromRequest;
use super::middleware::BoxFuture;
use super::request::Request;
use super::response::{Respond, Response};
use super::routing::HandlerFn;

pub trait Handler<T>: Clone + Send + Sync + 'static {
    fn call(&self, req: Request) -> BoxFuture<Response>;
}

pub fn into_handler_fn<H, T>(handler: H) -> HandlerFn
where
    H: Handler<T>,
{
    let handler = Arc::new(handler);
    Arc::new(move |req| {
        let handler = Arc::clone(&handler);
        Box::pin(async move { handler.call(req).await })
    })
}

/// 标记：无提取器。
pub struct NoEx;

/// 标记：同步闭包（Laravel `function () { return 'Hello'; }`）。
pub struct SyncFn;

/// 标记：带提取器的同步闭包。
pub struct SyncFnEx<T>(PhantomData<T>);

impl<F, Fut, R> Handler<NoEx> for F
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: Respond,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let fut = (self)();
        Box::pin(async move { fut.await.respond(&req) })
    }
}

impl<F, Fut, R, E1> Handler<(E1,)> for F
where
    F: Fn(E1) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: Respond,
    E1: FromRequest + Send + 'static,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let handler = self.clone();
        Box::pin(async move {
            let e1 = match E1::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            handler(e1).await.respond(&req)
        })
    }
}

impl<F, Fut, R, E1, E2> Handler<(E1, E2)> for F
where
    F: Fn(E1, E2) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: Respond,
    E1: FromRequest + Send + 'static,
    E2: FromRequest + Send + 'static,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let handler = self.clone();
        Box::pin(async move {
            let e1 = match E1::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e2 = match E2::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            handler(e1, e2).await.respond(&req)
        })
    }
}

impl<F, Fut, R, E1, E2, E3> Handler<(E1, E2, E3)> for F
where
    F: Fn(E1, E2, E3) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: Respond,
    E1: FromRequest + Send + 'static,
    E2: FromRequest + Send + 'static,
    E3: FromRequest + Send + 'static,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let handler = self.clone();
        Box::pin(async move {
            let e1 = match E1::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e2 = match E2::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e3 = match E3::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            handler(e1, e2, e3).await.respond(&req)
        })
    }
}

impl<F, Fut, R, E1, E2, E3, E4> Handler<(E1, E2, E3, E4)> for F
where
    F: Fn(E1, E2, E3, E4) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: Respond,
    E1: FromRequest + Send + 'static,
    E2: FromRequest + Send + 'static,
    E3: FromRequest + Send + 'static,
    E4: FromRequest + Send + 'static,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let handler = self.clone();
        Box::pin(async move {
            let e1 = match E1::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e2 = match E2::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e3 = match E3::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e4 = match E4::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            handler(e1, e2, e3, e4).await.respond(&req)
        })
    }
}

impl<F, Fut, R, E1, E2, E3, E4, E5> Handler<(E1, E2, E3, E4, E5)> for F
where
    F: Fn(E1, E2, E3, E4, E5) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: Respond,
    E1: FromRequest + Send + 'static,
    E2: FromRequest + Send + 'static,
    E3: FromRequest + Send + 'static,
    E4: FromRequest + Send + 'static,
    E5: FromRequest + Send + 'static,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let handler = self.clone();
        Box::pin(async move {
            let e1 = match E1::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e2 = match E2::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e3 = match E3::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e4 = match E4::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e5 = match E5::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            handler(e1, e2, e3, e4, e5).await.respond(&req)
        })
    }
}

impl<F, R> Handler<SyncFn> for F
where
    F: Fn() -> R + Clone + Send + Sync + 'static,
    R: Respond,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let handler = self.clone();
        Box::pin(async move { handler().respond(&req) })
    }
}

impl<F, R, E1> Handler<SyncFnEx<(E1,)>> for F
where
    F: Fn(E1) -> R + Clone + Send + Sync + 'static,
    R: Respond,
    E1: FromRequest + Send + 'static,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let handler = self.clone();
        Box::pin(async move {
            let e1 = match E1::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            handler(e1).respond(&req)
        })
    }
}

impl<F, R, E1, E2> Handler<SyncFnEx<(E1, E2)>> for F
where
    F: Fn(E1, E2) -> R + Clone + Send + Sync + 'static,
    R: Respond,
    E1: FromRequest + Send + 'static,
    E2: FromRequest + Send + 'static,
{
    fn call(&self, req: Request) -> BoxFuture<Response> {
        let handler = self.clone();
        Box::pin(async move {
            let e1 = match E1::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            let e2 = match E2::from_request(&req) {
                Ok(v) => v,
                Err(resp) => return resp,
            };
            handler(e1, e2).respond(&req)
        })
    }
}

/// 用于在泛型位置帮助推断提取器元组（宏内部用）。
pub struct HandlerMarker<T>(PhantomData<T>);

#[cfg(test)]
mod tests {
    use http::StatusCode;

    use super::*;
    use crate::core::controller::text;
    use crate::core::error::AppError;
    use crate::core::routing::Route;
    use crate::core::test_client::TestClient;

    #[tokio::test]
    async fn no_extractor_handler_uses_the_respond_error_boundary() {
        async fn handler() -> Result<Response, AppError> {
            Err(AppError::NotFound)
        }

        let mut client = TestClient::new(Route::get("/missing", handler).register());
        let response = client.get("/missing").await;

        assert_eq!(response.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn no_extractor_handler_still_accepts_plain_responses() {
        async fn handler() -> Response {
            text("ok")
        }

        let mut client = TestClient::new(Route::get("/", handler).register());
        assert_eq!(client.get("/").await.text(), "ok");
    }

    #[tokio::test]
    async fn sync_closure_returns_plain_text() {
        let mut client = TestClient::new(Route::get("/greeting", || "Hello World").register());
        let response = client.get("/greeting").await;
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.text(), "Hello World");
    }

    #[tokio::test]
    async fn sync_closure_can_read_path_params() {
        let mut client = TestClient::new(
            Route::get("/hi/:name", |req: Request| {
                format!("Hello {}", req.param_or("name", "world"))
            })
            .register(),
        );
        assert_eq!(client.get("/hi/namix").await.text(), "Hello namix");
    }
}
