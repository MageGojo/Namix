//! 把「带提取器的异步函数」收成统一的 HandlerFn。

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use super::extract::FromRequest;
use super::middleware::BoxFuture;
use super::request::Request;
use super::response::{IntoResponse, Respond, Response};
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

impl<F, Fut, R> Handler<NoEx> for F
where
    F: Fn() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send + 'static,
    R: IntoResponse,
{
    fn call(&self, _req: Request) -> BoxFuture<Response> {
        let fut = (self)();
        Box::pin(async move { fut.await.into_response() })
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

/// 用于在泛型位置帮助推断提取器元组（宏内部用）。
pub struct HandlerMarker<T>(PhantomData<T>);
