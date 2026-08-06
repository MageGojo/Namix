//! 事件分发器（Laravel Event 风格）。
//!
//! 注册页 `dispatch` → 各功能 `listen` 处理 → 把结果汇总回注册页。
//!
//! ```ignore
//! // 功能侧
//! event::listen(|e: &UserRegistered| {
//!     Reply::ok(format!("welcome {}", e.username))
//! });
//!
//! // 注册页
//! let outcome = event::dispatch(UserRegistered { username });
//! if outcome.all_ok() { /* 展示 outcome.summary() */ }
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

type BoxedHandler = Arc<dyn Fn(&(dyn Any + Send + Sync)) -> Reply + Send + Sync>;

static BUS: OnceLock<RwLock<HashMap<TypeId, Vec<BoxedHandler>>>> = OnceLock::new();

fn bus() -> &'static RwLock<HashMap<TypeId, Vec<BoxedHandler>>> {
    BUS.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 监听器回给发起方的一条结果。
#[derive(Debug, Clone)]
pub struct Reply {
    pub ok: bool,
    pub message: String,
}

impl Reply {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: message.into(),
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
        }
    }
}

/// 可转成 [`Reply`]（方便 `listen` 直接返回 `String` / `Result`）。
pub trait IntoReply {
    fn into_reply(self) -> Reply;
}

impl IntoReply for Reply {
    fn into_reply(self) -> Reply {
        self
    }
}

impl IntoReply for () {
    fn into_reply(self) -> Reply {
        Reply::ok("")
    }
}

impl IntoReply for String {
    fn into_reply(self) -> Reply {
        Reply::ok(self)
    }
}

impl IntoReply for &'static str {
    fn into_reply(self) -> Reply {
        Reply::ok(self)
    }
}

impl IntoReply for Result<String, String> {
    fn into_reply(self) -> Reply {
        match self {
            Ok(m) => Reply::ok(m),
            Err(m) => Reply::err(m),
        }
    }
}

impl IntoReply for Result<(), String> {
    fn into_reply(self) -> Reply {
        match self {
            Ok(()) => Reply::ok(""),
            Err(m) => Reply::err(m),
        }
    }
}

/// 一次 `dispatch` 的汇总结果（给注册页看）。
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub replies: Vec<Reply>,
}

impl Outcome {
    pub fn all_ok(&self) -> bool {
        self.replies.iter().all(|r| r.ok)
    }

    pub fn messages(&self) -> Vec<&str> {
        self.replies
            .iter()
            .filter(|r| !r.message.is_empty())
            .map(|r| r.message.as_str())
            .collect()
    }

    pub fn summary(&self) -> String {
        self.messages().join(" · ")
    }

    pub fn first_error(&self) -> Option<&str> {
        self.replies
            .iter()
            .find(|r| !r.ok)
            .map(|r| r.message.as_str())
    }
}

/// 注册监听器：某个功能「接到事件 → 做事 → 回执」。
pub fn listen<E, F, R>(f: F)
where
    E: Send + Sync + 'static,
    F: Fn(&E) -> R + Send + Sync + 'static,
    R: IntoReply,
{
    let handler: BoxedHandler = Arc::new(move |any| {
        let event = any
            .downcast_ref::<E>()
            .expect("event type mismatch in dispatcher");
        f(event).into_reply()
    });
    bus()
        .write()
        .expect("event bus")
        .entry(TypeId::of::<E>())
        .or_default()
        .push(handler);
}

/// 派发事件，同步收集所有监听器回执。
pub fn dispatch<E>(event: E) -> Outcome
where
    E: Send + Sync + 'static,
{
    let handlers = bus()
        .read()
        .expect("event bus")
        .get(&TypeId::of::<E>())
        .cloned()
        .unwrap_or_default();

    let boxed: Arc<dyn Any + Send + Sync> = Arc::new(event);
    let mut replies = Vec::with_capacity(handlers.len());
    for h in handlers {
        replies.push(h(boxed.as_ref()));
    }
    Outcome { replies }
}

/// 测试用：清空全部监听器。
pub fn clear() {
    if let Some(bus) = BUS.get() {
        bus.write().expect("event bus").clear();
    }
}

/// Async listener used for I/O-bound side effects. It can run in a request,
/// Action, or a queue worker without blocking synchronous `dispatch` callers.
pub type AsyncReplyFuture = std::pin::Pin<Box<dyn std::future::Future<Output = Reply> + Send>>;
type BoxedAsyncHandler = Arc<dyn Fn(Arc<dyn Any + Send + Sync>) -> AsyncReplyFuture + Send + Sync>;
static ASYNC_BUS: OnceLock<RwLock<HashMap<TypeId, Vec<BoxedAsyncHandler>>>> = OnceLock::new();
fn async_bus() -> &'static RwLock<HashMap<TypeId, Vec<BoxedAsyncHandler>>> {
    ASYNC_BUS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn listen_async<E, F, Fut, R>(listener: F)
where
    E: Send + Sync + 'static,
    F: Fn(Arc<E>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: IntoReply + Send + 'static,
{
    let listener = Arc::new(listener);
    let handler: BoxedAsyncHandler = Arc::new(move |event| {
        let event = Arc::downcast::<E>(event).expect("event type mismatch in async dispatcher");
        let listener = Arc::clone(&listener);
        Box::pin(async move { listener(event).await.into_reply() })
    });
    async_bus()
        .write()
        .expect("async event bus")
        .entry(TypeId::of::<E>())
        .or_default()
        .push(handler);
}

/// Dispatch all async listeners concurrently and collect their replies.
pub async fn dispatch_async<E>(event: E) -> Outcome
where
    E: Send + Sync + 'static,
{
    let handlers = async_bus()
        .read()
        .expect("async event bus")
        .get(&TypeId::of::<E>())
        .cloned()
        .unwrap_or_default();
    let event: Arc<dyn Any + Send + Sync> = Arc::new(event);
    let replies = futures_util::future::join_all(
        handlers
            .into_iter()
            .map(|handler| handler(Arc::clone(&event))),
    )
    .await;
    Outcome { replies }
}
