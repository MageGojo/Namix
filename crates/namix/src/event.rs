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
use std::fmt::Display;
use std::sync::{Arc, OnceLock, RwLock};

use thiserror::Error;

use crate::queue::{Job, JobFuture};

type BoxedHandler = Arc<dyn Fn(&(dyn Any + Send + Sync)) -> Reply + Send + Sync>;

static BUS: OnceLock<RwLock<HashMap<TypeId, Vec<BoxedHandler>>>> = OnceLock::new();

fn bus() -> &'static RwLock<HashMap<TypeId, Vec<BoxedHandler>>> {
    BUS.get_or_init(|| RwLock::new(HashMap::new()))
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum EventError {
    #[error("{bus} event bus lock poisoned")]
    BusLockPoisoned { bus: &'static str },
    #[error("event listeners failed: {messages:?}")]
    ListenerFailures { messages: Vec<String> },
}

pub type EventResult<T> = Result<T, EventError>;

impl From<EventError> for crate::AppError {
    fn from(error: EventError) -> Self {
        Self::internal(error)
    }
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

impl<T, E> IntoReply for Result<T, E>
where
    T: IntoReply,
    E: Display,
{
    fn into_reply(self) -> Reply {
        match self {
            Ok(value) => value.into_reply(),
            Err(error) => Reply::err(error.to_string()),
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

    /// Convert listener-level failures into a typed boundary suitable for
    /// queue workers, Actions, and service code.
    pub fn ensure_ok(self) -> EventResult<Self> {
        let messages = self
            .replies
            .iter()
            .filter(|reply| !reply.ok)
            .map(|reply| reply.message.clone())
            .collect::<Vec<_>>();
        if messages.is_empty() {
            Ok(self)
        } else {
            Err(EventError::ListenerFailures { messages })
        }
    }
}

/// 注册监听器：某个功能「接到事件 → 做事 → 回执」。
pub fn listen<E, F, R>(f: F)
where
    E: Send + Sync + 'static,
    F: Fn(&E) -> R + Send + Sync + 'static,
    R: IntoReply,
{
    if let Err(error) = try_listen(f) {
        tracing::error!(error = ?error, "event listener registration failed");
    }
}

/// Register a synchronous listener while preserving infrastructure errors.
pub fn try_listen<E, F, R>(f: F) -> EventResult<()>
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
        .map_err(|_| EventError::BusLockPoisoned { bus: "sync" })?
        .entry(TypeId::of::<E>())
        .or_default()
        .push(handler);
    Ok(())
}

/// 派发事件，同步收集所有监听器回执。
pub fn dispatch<E>(event: E) -> Outcome
where
    E: Send + Sync + 'static,
{
    match try_dispatch(event) {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!(error = ?error, "event dispatch failed");
            Outcome {
                replies: vec![Reply::err(error.to_string())],
            }
        }
    }
}

/// Dispatch an event and distinguish a bus failure from an event with no
/// registered listeners.
pub fn try_dispatch<E>(event: E) -> EventResult<Outcome>
where
    E: Send + Sync + 'static,
{
    let handlers = bus()
        .read()
        .map_err(|_| EventError::BusLockPoisoned { bus: "sync" })?
        .get(&TypeId::of::<E>())
        .cloned()
        .unwrap_or_default();

    let boxed: Arc<dyn Any + Send + Sync> = Arc::new(event);
    let mut replies = Vec::with_capacity(handlers.len());
    for h in handlers {
        replies.push(h(boxed.as_ref()));
    }
    Ok(Outcome { replies })
}

/// 测试用：清空全部监听器。
pub fn clear() {
    if let Err(error) = try_clear() {
        tracing::error!(error = ?error, "event bus clear failed");
    }
}

pub fn try_clear() -> EventResult<()> {
    if let Some(bus) = BUS.get() {
        bus.write()
            .map_err(|_| EventError::BusLockPoisoned { bus: "sync" })?
            .clear();
    }
    if let Some(bus) = ASYNC_BUS.get() {
        bus.write()
            .map_err(|_| EventError::BusLockPoisoned { bus: "async" })?
            .clear();
    }
    Ok(())
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
    if let Err(error) = try_listen_async(listener) {
        tracing::error!(error = ?error, "async event listener registration failed");
    }
}

pub fn try_listen_async<E, F, Fut, R>(listener: F) -> EventResult<()>
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
        .map_err(|_| EventError::BusLockPoisoned { bus: "async" })?
        .entry(TypeId::of::<E>())
        .or_default()
        .push(handler);
    Ok(())
}

/// Dispatch all async listeners concurrently and collect their replies.
pub async fn dispatch_async<E>(event: E) -> Outcome
where
    E: Send + Sync + 'static,
{
    match try_dispatch_async(event).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!(error = ?error, "async event dispatch failed");
            Outcome {
                replies: vec![Reply::err(error.to_string())],
            }
        }
    }
}

pub async fn try_dispatch_async<E>(event: E) -> EventResult<Outcome>
where
    E: Send + Sync + 'static,
{
    let handlers = async_bus()
        .read()
        .map_err(|_| EventError::BusLockPoisoned { bus: "async" })?
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
    Ok(Outcome { replies })
}

/// A queued event dispatches the registered asynchronous listeners when a
/// worker executes it. Listener failures retain their event context in the
/// queue's `anyhow` error chain.
pub struct QueuedEvent<E> {
    event: E,
}

impl<E> QueuedEvent<E> {
    pub fn new(event: E) -> Self {
        Self { event }
    }
}

impl<E> Job for QueuedEvent<E>
where
    E: Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        std::any::type_name::<E>()
    }

    fn handle(self: Box<Self>) -> JobFuture {
        Box::pin(async move {
            try_dispatch_async(self.event).await?.ensure_ok()?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Queue;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct TypedFailure;

    #[derive(Debug, Error)]
    #[error("typed listener failure")]
    struct ListenerError;

    #[test]
    fn typed_listener_errors_reach_the_outcome_boundary() {
        try_listen(|_: &TypedFailure| -> Result<(), ListenerError> { Err(ListenerError) }).unwrap();
        let error = try_dispatch(TypedFailure).unwrap().ensure_ok().unwrap_err();
        assert!(matches!(error, EventError::ListenerFailures { .. }));
        assert!(error.to_string().contains("typed listener failure"));
    }

    struct BackgroundEvent(Arc<AtomicBool>);

    #[tokio::test]
    async fn asynchronous_listener_can_run_as_a_queue_job() {
        try_listen_async(|event: Arc<BackgroundEvent>| async move {
            event.0.store(true, Ordering::SeqCst);
        })
        .unwrap();

        let hit = Arc::new(AtomicBool::new(false));
        let queue = Queue::memory(1);
        queue
            .dispatch(QueuedEvent::new(BackgroundEvent(Arc::clone(&hit))))
            .await
            .unwrap();
        let (name, result) = queue.work_once().await.unwrap();
        result.unwrap();
        assert!(name.contains("BackgroundEvent"));
        assert!(hit.load(Ordering::SeqCst));
    }

    struct FailingBackgroundEvent;

    #[tokio::test]
    async fn queued_event_reports_listener_failure_to_the_worker() {
        try_listen_async(|_: Arc<FailingBackgroundEvent>| async { Err::<(), _>(ListenerError) })
            .unwrap();
        let queue = Queue::memory(1);
        queue
            .dispatch(QueuedEvent::new(FailingBackgroundEvent))
            .await
            .unwrap();
        let (_, result) = queue.work_once().await.unwrap();
        assert!(format!("{:#}", result.unwrap_err()).contains("typed listener failure"));
    }
}
