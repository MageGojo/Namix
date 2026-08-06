//! Namix 框架门面。
//!
//! CLI：`-p/--port` `-h/--lan` `--https` `--https-port` `--help`

pub mod authorization;
pub mod boot;
pub mod cache;
pub mod config;
pub mod event;
pub mod log;
pub mod mail;
pub mod notification;
pub mod observability;
pub mod pagination;
pub mod queue;
pub mod runtime;
pub mod sms;
pub mod storage;

#[cfg(any(
    feature = "sqlite",
    feature = "postgresql",
    feature = "mysql",
    feature = "turso",
    feature = "dynamodb"
))]
pub mod db;

pub use namix_http as http_impl;
pub use namix_macros::{
    FormField, NamedRoute, ViewData, ViewProps, route, route_names, routes, server,
};

pub use namix_http::http;
pub use namix_http::routing_path_join;
pub use namix_http::{
    ActionError, ActionOk, ActionRateLimits, AppError, ByteRange, ClientIp, ContentRange,
    ContentRangeHeader, ContentType, Controller, CookieOptions, CsrfConfig, CsrfProtection,
    CsrfToken, FLASH_COOKIE, Flash, FromRequest, IntoActionResponse, IntoResponse, Json,
    MiddlewareFn, Next, Path, RangeParseError, RateLimitPolicy, RateLimitScope, RateLimiter,
    Request, ResourceController, ResourceFuture, Respond, Response, Route, Router, Server,
    ServerFn, Sse, SseClosed, SseEvent, SseSender, TestClient, TestResponse, TestWebSocket,
    TlsConfig, UploadProgress, WsError, WsMessage, WsReceiver, WsSender, WsSocket, controller,
    csrf, extract, is_local_path, middleware, rate_limit, resource, resources, routing, server_fn,
    set_user_subject, sse, transfer, validate, wrap_middleware, ws,
};

pub use namix_http::routing::{IntoRouteName, RouteCatalog, RouteExport};
pub use namix_http::validate::{
    Field, FormRedirect, FormRequest, Rule, Validated, ValidationError, Validator,
};

#[cfg(feature = "pages")]
pub use namix_http::pages::{self, RenderMode, View, ViewBag, ViewPage, view};

pub use authorization::{Ability, Gate, Policy, authorize};
pub use boot::Boot;
pub use cache::{Cache, CacheStore, MemoryCache, RedisBackend, RedisCache};
pub use config::{
    ConfigError, MailSection, NamixToml, RateLimitSection, SecuritySection, SmsSection,
};
pub use event::{
    AsyncReplyFuture, IntoReply, Outcome, Reply, dispatch, dispatch_async, listen, listen_async,
};
pub use mail::{Mail, MailError, MailMessage, MailResult};
pub use notification::{
    LogNotificationDriver, Notification, NotificationChannel, NotificationDriver, Notifier,
};
pub use observability::{
    RequestId, measure_db, measure_db_async, request_id, request_id_middleware,
};
pub use pagination::{Paginator, QueryOptions, SortField, SortWhitelist};
pub use queue::{Job, JobFuture, JobResult, Queue};
pub use runtime::{init_workdir, resolve_home};
pub use sms::{Sms, SmsError, SmsMessage, SmsResult};
pub use storage::{
    LocalStorage, S3CompatibleStorage, S3Transport, Storage, StorageDriver, StorageError,
    StorageResult, TemporaryUrl, UploadPolicy,
};

pub mod prelude {
    pub use crate::controller::{
        download, download_as, download_data, file, html, json, json_raw, no_content, not_found,
        raw, redirect, redirect_permanent, text, with_status,
    };
    pub use crate::event::{self, Outcome, Reply, dispatch, dispatch_async, listen, listen_async};
    pub use crate::extract::{FromRequest, Json, Path};
    pub use crate::log;
    pub use crate::middleware::Next;
    pub use crate::routing::{IntoRouteName, NamedRoute};
    pub use crate::validate::{
        Field, FormRedirect, FormRequest, Rule, Validated, ValidationError, Validator,
    };
    pub use crate::{
        Ability, ActionError, ActionOk, AppError, Boot, ByteRange, Cache, ContentRange,
        ContentType, CookieOptions, CsrfToken, Flash, FormField, Gate, IntoActionResponse,
        IntoResponse, JobResult, Mail, MailError, MailMessage, MailResult, NamixToml,
        RangeParseError, RateLimitPolicy, RateLimitScope, RateLimiter, Request, RequestId,
        ResourceController, ResourceFuture, Respond, Response, Route, Router, Server, Sms,
        SmsError, SmsMessage, SmsResult, Sse, SseEvent, SseSender, Storage, StorageError,
        StorageResult, TestClient, TestResponse, TestWebSocket, TlsConfig, UploadPolicy,
        UploadProgress, WsMessage, WsReceiver, WsSender, WsSocket, authorize, is_local_path,
        resource, resources, route, route_names, routes, server,
    };
    // flash / redirect_*（core）—— 匿名导入，保证方法在作用域
    pub use crate::controller::Controller as _;
    // view / render / with（pages 开启时覆盖为完整 Controller）
    #[cfg(feature = "pages")]
    pub use crate::Controller;
    #[cfg(not(feature = "pages"))]
    pub use crate::Controller;
    #[cfg(feature = "pages")]
    pub use crate::{RenderMode, View, ViewBag, ViewData, ViewPage, ViewProps, view};
    #[cfg(feature = "pages")]
    pub use serde_json::json;
}
