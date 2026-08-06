//! 请求提取器：`Path` / `Json` / `Request`。

use std::str::FromStr;

use http::StatusCode;
use serde::de::DeserializeOwned;

use super::request::Request;
use super::response::{IntoResponse, Response};

pub trait FromRequest: Sized {
    // `Response` is intentionally returned directly so handlers can short-circuit
    // without a second error-to-response conversion or public boxed error type.
    #[allow(clippy::result_large_err)]
    fn from_request(req: &Request) -> Result<Self, Response>;
}

impl FromRequest for Request {
    fn from_request(req: &Request) -> Result<Self, Response> {
        Ok(req.clone())
    }
}

/// 路径参数。按路由里 `:param` 的定义顺序提取第一个。
pub struct Path<T>(pub T);

impl<T> FromRequest for Path<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    fn from_request(req: &Request) -> Result<Self, Response> {
        let raw = req
            .params()
            .first()
            .map(|(_, v)| v.as_str())
            .ok_or_else(|| {
                (StatusCode::BAD_REQUEST, "missing path param".into()).into_response()
            })?;
        let value = raw.parse::<T>().map_err(|e| {
            (StatusCode::BAD_REQUEST, format!("invalid path param: {e}")).into_response()
        })?;
        Ok(Path(value))
    }
}

/// JSON 请求体。
pub struct Json<T>(pub T);

impl<T> FromRequest for Json<T>
where
    T: DeserializeOwned,
{
    fn from_request(req: &Request) -> Result<Self, Response> {
        serde_json::from_slice(req.body())
            .map(Json)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid json: {e}")).into_response())
    }
}
