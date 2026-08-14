//! Namix 框架门面。
//!
//! CLI：`-p/--port` `-h/--lan` `--https` `--https-port` `--help`

pub mod access;
pub mod auth_view;
pub mod authorization;
pub mod boot;
pub mod cache;
pub mod config;
pub mod crypt;
pub mod event;
pub mod i18n;
pub mod jwt;
pub mod log;
pub mod mail;
pub mod notification;
pub mod oauth;
pub mod observability;
pub mod one_time_token;
pub mod pagination;
pub mod presence;
pub mod queue;
pub mod queue_durable;
pub mod runtime;
pub mod session;
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
    CsrfToken, ErrorPage, ErrorPages, FLASH_COOKIE, Flash, FromRequest, IntoActionResponse,
    IntoResponse, Json, JsonBodyError, MiddlewareFn, MultipartBag, Next, OrNotFound, Path,
    RangeParseError, RateLimitPolicy, RateLimitScope, RateLimiter, Request, ResourceController,
    ResourceFuture, Respond, Response, Route, Router, Server, ServerFn, Sse, SseClosed, SseEvent,
    SseSender, TestClient, TestClientError, TestClientResult, TestResponse, TestWebSocket,
    TlsConfig, TrustedProxies, TrustedProxyError, UploadProgress, UploadedFile, WsError, WsMessage,
    WsReceiver, WsSender, WsSocket, controller, csrf, extract, is_local_path, middleware,
    rate_limit, resource, resources, routing, server_fn, set_user_subject, sse, transfer, validate,
    wrap_middleware, ws,
};

pub use namix_http::routing::{IntoRouteName, RouteCatalog, RouteExport};
pub use namix_http::validate::{
    Field, FormRedirect, FormRequest, PresenceVerifier, Rule, Validated, ValidationError,
    Validator, clear_error_translator, clear_presence_verifier, install_error_translator,
    install_presence_verifier, translate_error,
};

#[cfg(feature = "pages")]
pub use namix_http::pages::{
    self, Document, DocumentTemplateError, RenderMode, View, ViewBag, ViewPage, view,
};

pub use access::{Access, allows as role_allows};
pub use auth_view::AuthView;
pub use authorization::{Ability, Gate, Policy, authorize};
pub use boot::Boot;
pub use cache::{
    Cache, CacheError, CacheResult, CacheStore, MemoryCache, RedisBackend, RedisCache,
};
pub use config::{
    ConfigError, DiskConfig, I18nSection, MailSection, NamixToml, QueueSection, RateLimitSection,
    SecuritySection, SessionSection, SmsSection, StorageSection,
};
pub use crypt::{Crypt, CryptError};
pub use event::{
    AsyncReplyFuture, EventError, EventResult, IntoReply, Outcome, QueuedEvent, Reply, dispatch,
    dispatch_async, listen, listen_async, try_clear, try_dispatch, try_dispatch_async, try_listen,
    try_listen_async,
};
pub use i18n::{trans, trans_error, trans_with};
pub use jwt::{Jwt, JwtClaims, JwtError};
pub use mail::{
    Mail, MailError, MailJob, MailMessage, MailResult, MailTransport, MailTransportError,
    MailTransportResult,
};
pub use notification::{
    LogNotificationDriver, MailNotificationDriver, Notification, NotificationChannel,
    NotificationDriver, NotificationError, NotificationJob, NotificationResult, NotificationRouter,
    NotificationTransportError, NotificationTransportResult, Notifier,
};
pub use oauth::{DevProvider, SocialProvider, SocialUser};
pub use observability::{
    RequestId, measure_db, measure_db_async, measure_db_result, measure_db_result_async,
    request_id, request_id_middleware,
};
pub use one_time_token::{OneTimeTokenStore, TokenStoreError};
pub use pagination::{Paginator, QueryOptions, SortField, SortWhitelist};
pub use queue::{Job, JobFuture, JobResult, Queue, QueueError, QueueResult};
pub use queue_durable::{
    DurableQueue, DurableQueueError, DurableQueueResult, JobRecord, QueuedJob,
    dispatch as dispatch_job, dispatch_later as dispatch_job_later, register_job,
};
pub use runtime::{init_workdir, resolve_home};
pub use session::{
    AuthSession, FileSessionStore, MemorySessionStore, RedisSessionStore, Session, SessionError,
    SessionResult, SessionStore,
};
pub use sms::{
    Sms, SmsError, SmsMessage, SmsResult, SmsTransport, SmsTransportError, SmsTransportResult,
};
pub use storage::{
    LocalStorage, MemoryStorage, ReadOnlyStorage, S3CompatibleStorage, S3Transport, ScopedStorage,
    Storage, StorageDriver, StorageError, StorageImage, StorageResult, TemporaryUrl, UploadPolicy,
    Visibility,
};

pub mod prelude {
    pub use crate::controller::{
        download, download_as, download_data, file, html, json, json_raw, no_content, not_found,
        raw, redirect, redirect_permanent, text, with_status,
    };
    pub use crate::event::{
        self, EventError, EventResult, Outcome, QueuedEvent, Reply, dispatch, dispatch_async,
        listen, listen_async, try_dispatch, try_dispatch_async, try_listen, try_listen_async,
    };
    pub use crate::extract::{FromRequest, Json, Path};
    pub use crate::log;
    pub use crate::middleware::Next;
    pub use crate::routing::{IntoRouteName, NamedRoute};
    pub use crate::validate::{
        Field, FormRedirect, FormRequest, Rule, Validated, ValidationError, Validator,
    };
    pub use crate::{
        Ability, Access, ActionError, ActionOk, AppError, AuthSession, AuthView, Boot, ByteRange,
        Cache, CacheError, CacheResult, ContentRange, ContentType, CookieOptions, Crypt, CsrfToken,
        DurableQueue, DurableQueueError, ErrorPage, ErrorPages, Flash, FormField, Gate,
        IntoActionResponse, IntoResponse, Job, JobFuture, JobResult, JsonBodyError, Mail,
        MailError, MailJob, MailMessage, MailResult, MailTransport, MultipartBag, NamixToml,
        Notification, NotificationChannel, NotificationDriver, NotificationError, NotificationJob,
        NotificationResult, NotificationRouter, Notifier, OneTimeTokenStore, OrNotFound, Policy,
        Queue, QueueError, QueueResult, QueuedJob, RangeParseError, RateLimitPolicy,
        RateLimitScope, RateLimiter, Request, RequestId, ResourceController, ResourceFuture,
        Respond, Response, Route, Router, Server, Session, Sms, SmsError, SmsMessage, SmsResult,
        Sse, SseEvent, SseSender, Storage, StorageError, StorageResult, TestClient,
        TestClientError, TestClientResult, TestResponse, TestWebSocket, TlsConfig, TrustedProxies,
        TrustedProxyError, UploadPolicy, UploadProgress, UploadedFile, Visibility, WsMessage,
        WsReceiver, WsSender, WsSocket, authorize, dispatch_job, dispatch_job_later, is_local_path,
        register_job, resource, resources, route, route_names, routes, server, trans, trans_error,
        trans_with,
    };
    pub use crate::{
        FileSessionStore, Jwt, JwtClaims, MemorySessionStore, RedisSessionStore, SessionStore,
    };
    // flash / redirect_*（core）—— 匿名导入，保证方法在作用域
    pub use crate::OrNotFound as _;
    pub use crate::controller::Controller as _;
    // view / render / with（pages 开启时覆盖为完整 Controller）
    #[cfg(feature = "pages")]
    pub use crate::Controller;
    #[cfg(not(feature = "pages"))]
    pub use crate::Controller;
    #[cfg(feature = "pages")]
    pub use crate::{
        Document, DocumentTemplateError, RenderMode, View, ViewBag, ViewData, ViewPage, ViewProps,
        view,
    };
    #[cfg(feature = "pages")]
    pub use serde_json::json;
}
