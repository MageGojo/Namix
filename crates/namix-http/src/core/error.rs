//! 统一业务错误边界。
//!
//! 控制器直接返回 `Result<Response, AppError>` 时，会按请求协商 HTML 或 JSON；
//! `#[server]` Action 则映射为同一份 `{ message, errors }` 契约。

use std::collections::HashMap;

use http::StatusCode;
use thiserror::Error;

use super::content_type::ContentType;
use super::request::Request;
use super::response::{BoxError, Respond, Response};
use super::validate::ValidationError;

/// 应用层可直接使用的标准错误。
///
/// 业务可返回明确的 HTTP 语义（如 `Validation` 或 `Forbidden`）。未知的
/// 基础设施错误经 [`AppError::internal`] 进入统一边界：调用方只看到通用
/// 500，日志保留完整 source chain 以便排障。
#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("authentication required")]
    Unauthenticated,
    #[error("forbidden")]
    Forbidden,
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("validation failed")]
    Validation(HashMap<String, String>),
    #[error("too many requests")]
    RateLimited { retry_after: u64 },
    #[error("internal server error")]
    Internal {
        #[source]
        source: BoxError,
    },
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn validation(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation(HashMap::from([(field.into(), message.into())]))
    }

    /// Wrap an unexpected error without losing its source chain.
    pub fn internal(error: impl Into<BoxError>) -> Self {
        Self::Internal {
            source: error.into(),
        }
    }

    /// Compatibility helper for string-only integrations. Prefer
    /// [`AppError::internal`] whenever a concrete error value exists.
    pub fn internal_message(message: impl Into<String>) -> Self {
        Self::internal(MessageError(message.into()))
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::BadRequest(message) | Self::Conflict(message) => message,
            Self::Unauthenticated => "authentication required",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not found",
            Self::Validation(_) => "validation failed",
            Self::RateLimited { .. } => "too many requests",
            Self::Internal { .. } => "internal server error",
        }
    }

    pub fn fields(&self) -> HashMap<String, String> {
        match self {
            Self::Validation(fields) => fields.clone(),
            _ => HashMap::from([("_".to_string(), self.message().to_string())]),
        }
    }

    /// 普通 HTTP 控制器的协商响应。
    pub fn into_response_for(self, req: &Request) -> Response {
        self.log_internal();
        let status = self.status();
        let message = self.message().to_string();
        let fields = self.fields();
        let retry_after = match self {
            Self::RateLimited { retry_after } => Some(retry_after),
            _ => None,
        };

        let mut response = if wants_json(req) {
            Response::new(
                status,
                ContentType::Json,
                serde_json::json!({
                    "error": message,
                    "message": message,
                    "errors": fields,
                })
                .to_string(),
            )
        } else {
            let title = status.canonical_reason().unwrap_or("Error");
            Response::new(
                status,
                ContentType::Html,
                format!(
                    "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>{}</title><main><h1>{}</h1><p>{}</p></main></html>",
                    escape_html(title),
                    escape_html(title),
                    escape_html(&message),
                ),
            )
        };
        if let Some(retry_after) = retry_after {
            response.set_header("retry-after", retry_after.to_string());
        }
        response
    }

    /// `#[server]` shares the same status and `{ error, message, errors }`
    /// contract as JSON HTTP controllers.
    pub fn into_action_response(self) -> Response {
        self.log_internal();
        let status = self.status();
        let message = self.message().to_string();
        let fields = self.fields();
        let retry_after = match self {
            Self::RateLimited { retry_after } => Some(retry_after),
            _ => None,
        };
        let mut response = Response::new(
            status,
            ContentType::Json,
            serde_json::json!({
                "error": message,
                "message": message,
                "errors": fields,
            })
            .to_string(),
        );
        if let Some(retry_after) = retry_after {
            response.set_header("retry-after", retry_after.to_string());
        }
        response
    }

    fn log_internal(&self) {
        if let Self::Internal { source } = self {
            tracing::error!(error = ?source, "namix internal application error");
        }
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct MessageError(String);

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self::internal_message(message)
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::internal_message(message)
    }
}

impl From<ValidationError> for AppError {
    fn from(error: ValidationError) -> Self {
        let mut fields = HashMap::new();
        for (field, messages) in error.errors() {
            if let Some(message) = messages.first() {
                fields.insert(field.clone(), message.clone());
            }
        }
        if fields.is_empty() {
            fields.insert("_".into(), "validation failed".into());
        }
        Self::Validation(fields)
    }
}

impl<T: Respond> Respond for Result<T, AppError> {
    fn respond(self, req: &Request) -> Response {
        match self {
            Ok(value) => value.respond(req),
            Err(error) => error.into_response_for(req),
        }
    }
}

pub(crate) fn wants_json(req: &Request) -> bool {
    req.path().starts_with("/api/")
        || req
            .header("accept")
            .is_some_and(|value| value.contains("application/json"))
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use http::{HeaderMap, Method, Uri};
    use std::error::Error as _;

    use super::*;

    fn request(accept: Option<&str>) -> Request {
        let mut headers = HeaderMap::new();
        if let Some(accept) = accept {
            headers.insert("accept", accept.parse().unwrap());
        }
        Request::new(
            Method::GET,
            Uri::from_static("/items"),
            headers,
            Bytes::new(),
        )
    }

    #[test]
    fn maps_validation_to_json_field_bag() {
        let response = AppError::validation("name", "required")
            .into_response_for(&request(Some("application/json")));
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response.content_type(),
            Some("application/json; charset=utf-8")
        );
    }

    #[test]
    fn maps_browser_errors_to_html() {
        let response = AppError::NotFound.into_response_for(&request(Some("text/html")));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.content_type(), Some("text/html; charset=utf-8"));
    }

    #[test]
    fn maps_rate_limit_to_action_json_and_retry_header() {
        let response = AppError::RateLimited { retry_after: 12 }.into_action_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.header("retry-after"), Some("12"));
    }

    #[test]
    fn internal_error_keeps_source_but_hides_it_from_callers() {
        let error = AppError::internal(std::io::Error::other("database password rejected"));
        assert_eq!(error.to_string(), "internal server error");
        assert_eq!(
            error.source().unwrap().to_string(),
            "database password rejected"
        );
        let response = error.into_action_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
