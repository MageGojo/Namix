//! Namix HTTP 实现层（`namix-http`）。
//!
//! 业务代码请依赖门面 crate `namix`，使用 `namix::prelude::*`。
//!
//! ```text
//! namix-http/
//!   core/       常驻：server / routing / middleware / controller / extract
//!   features/   可选：由 app/namix.toml 开关编译
//! ```

pub mod core;
pub mod features;

pub use core::content_type::ContentType;
#[cfg(not(feature = "pages"))]
pub use core::controller::Controller;
pub use core::controller::{self, FLASH_COOKIE, Flash};
pub use core::crypt;
pub use core::csrf::{CsrfConfig, CsrfProtection, CsrfToken};
pub use core::error::{AppError, OrNotFound};
pub use core::error_pages::{ErrorPage, ErrorPages};
pub use core::extract::{FromRequest, Json, Path};
pub use core::middleware::{MiddlewareFn, Next, wrap_middleware};
pub use core::proxy::{TrustedProxies, TrustedProxyError};
pub use core::rate_limit::{RateLimitPolicy, RateLimitScope, RateLimiter, set_user_subject};
pub use core::request::{ClientIp, JsonBodyError, Request, is_local_path};
pub use core::response::{CookieOptions, IntoResponse, Respond, Response};
pub use core::routing::{
    IntoRouteName, NamedRoute, ResourceController, ResourceFuture, Route, RouteCatalog,
    RouteExport, Router, path_join as routing_path_join, resource, resources,
};
pub use core::server::{Server, TlsConfig};
pub use core::server_fn::{
    self, ActionError, ActionOk, ActionRateLimits, IntoActionResponse, ServerFn,
};
pub use core::sse::{self, Sse, SseClosed, SseEvent, SseSender};
pub use core::test_client::{
    TestClient, TestClientError, TestClientResult, TestResponse, TestWebSocket,
};
pub use core::transfer::{
    self, ByteRange, ContentRange, ContentRangeHeader, RangeParseError, UploadProgress,
};
pub use core::upload::{MultipartBag, UploadedFile};
pub use core::validate::{
    self, Field, PresenceVerifier, Rule, Validated, ValidationError, Validator,
    clear_error_translator, clear_presence_verifier, install_error_translator,
    install_presence_verifier, translate_error,
};
pub use core::ws::{self, WsError, WsMessage, WsReceiver, WsSender, WsSocket};
#[cfg(feature = "pages")]
pub use features::pages::Controller;
pub use http;

#[cfg(feature = "pages")]
pub mod pages {
    pub use crate::features::pages::*;
}

pub mod routing {
    pub use crate::core::routing::*;
}

pub mod middleware {
    pub use crate::core::middleware::*;
}

pub mod csrf {
    pub use crate::core::csrf::*;
}

pub mod rate_limit {
    pub use crate::core::rate_limit::*;
}

pub mod extract {
    pub use crate::core::extract::*;
}
