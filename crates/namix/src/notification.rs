//! Channel-neutral notifications.
//!
//! Development can use [`LogNotificationDriver`]. Production integrations
//! implement [`NotificationDriver`] and are composed per channel with
//! [`NotificationRouter`]. Both direct and queued delivery expose typed
//! framework errors while retaining the transport's source chain.

use std::collections::HashMap;
use std::error::Error as StdError;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::mail::{Mail, MailMessage};
use crate::queue::{Job, JobFuture, Queue, QueueResult};

pub type NotificationTransportError = Box<dyn StdError + Send + Sync + 'static>;
pub type NotificationTransportResult<T> = Result<T, NotificationTransportError>;
pub type NotificationResult<T> = Result<T, NotificationError>;

#[derive(Debug, Error)]
pub enum NotificationError {
    #[error("notification data serialization failed")]
    Serialize(#[source] serde_json::Error),
    #[error("notification transport `{driver}` failed")]
    Transport {
        driver: String,
        #[source]
        source: NotificationTransportError,
    },
    #[error("notification log lock poisoned")]
    LogLockPoisoned,
}

impl From<NotificationError> for crate::AppError {
    fn from(error: NotificationError) -> Self {
        Self::internal(error)
    }
}

impl NotificationError {
    pub fn transport(
        driver: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::Transport {
            driver: driver.into(),
            source: Box::new(source),
        }
    }
}

#[derive(Clone, Debug, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    Mail,
    Sms,
    Database,
    Webhook,
    Push,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub channel: NotificationChannel,
    pub recipient: String,
    pub title: String,
    pub body: String,
    pub data: serde_json::Value,
}

impl Notification {
    pub fn new(
        channel: NotificationChannel,
        recipient: impl Into<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            channel,
            recipient: recipient.into(),
            title: title.into(),
            body: body.into(),
            data: serde_json::Value::Null,
        }
    }

    /// Compatibility builder for infallible/common serde values. Use
    /// [`Notification::try_data`] when serialization failure is actionable.
    pub fn data(mut self, data: impl Serialize) -> Self {
        match serde_json::to_value(data) {
            Ok(data) => self.data = data,
            Err(error) => {
                tracing::error!(error = ?error, "notification data serialization failed");
            }
        }
        self
    }

    pub fn try_data(mut self, data: impl Serialize) -> NotificationResult<Self> {
        self.data = serde_json::to_value(data).map_err(NotificationError::Serialize)?;
        Ok(self)
    }
}

/// Transport contract for SMTP/API/push/chat integrations.
///
/// The boxed source is intentionally an actual `Error`, not a string. This
/// keeps provider error chains available to tracing and queue workers without
/// forcing the framework to depend on a specific client library.
pub trait NotificationDriver: Send + Sync + 'static {
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    fn send(&self, notification: &Notification) -> NotificationTransportResult<()>;
}

#[derive(Clone)]
pub struct Notifier {
    driver: Arc<dyn NotificationDriver>,
}

impl Notifier {
    pub fn new(driver: impl NotificationDriver) -> Self {
        Self {
            driver: Arc::new(driver),
        }
    }

    pub fn send(&self, notification: Notification) -> NotificationResult<()> {
        self.driver
            .send(&notification)
            .map_err(|source| NotificationError::Transport {
                driver: self.driver.name().to_owned(),
                source,
            })
    }

    pub fn job(&self, notification: Notification) -> NotificationJob {
        NotificationJob::new(self.clone(), notification)
    }

    pub async fn dispatch(&self, queue: &Queue, notification: Notification) -> QueueResult<()> {
        queue.dispatch(self.job(notification)).await
    }
}

/// Route each notification channel to a separately replaceable production
/// transport.
#[derive(Clone, Default)]
pub struct NotificationRouter {
    drivers: HashMap<NotificationChannel, Arc<dyn NotificationDriver>>,
}

impl NotificationRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn route(mut self, channel: NotificationChannel, driver: impl NotificationDriver) -> Self {
        self.drivers.insert(channel, Arc::new(driver));
        self
    }
}

#[derive(Debug, Error)]
#[error("no notification transport registered for channel {channel:?}")]
struct MissingChannelDriver {
    channel: NotificationChannel,
}

impl NotificationDriver for NotificationRouter {
    fn name(&self) -> &'static str {
        "notification-router"
    }

    fn send(&self, notification: &Notification) -> NotificationTransportResult<()> {
        let driver = self.drivers.get(&notification.channel).ok_or_else(|| {
            Box::new(MissingChannelDriver {
                channel: notification.channel.clone(),
            }) as NotificationTransportError
        })?;
        driver.send(notification)
    }
}

#[derive(Clone, Default)]
pub struct LogNotificationDriver {
    sent: Arc<Mutex<Vec<Notification>>>,
}

impl LogNotificationDriver {
    pub fn try_sent(&self) -> NotificationResult<Vec<Notification>> {
        self.sent
            .lock()
            .map(|sent| sent.clone())
            .map_err(|_| NotificationError::LogLockPoisoned)
    }

    pub fn sent(&self) -> Vec<Notification> {
        match self.try_sent() {
            Ok(sent) => sent,
            Err(error) => {
                tracing::error!(error = ?error, "notification log read failed");
                Vec::new()
            }
        }
    }
}

impl NotificationDriver for LogNotificationDriver {
    fn name(&self) -> &'static str {
        "log"
    }

    fn send(&self, notification: &Notification) -> NotificationTransportResult<()> {
        tracing::info!(
            channel = ?notification.channel,
            to = %notification.recipient,
            title = %notification.title,
            "notification sent"
        );
        self.sent
            .lock()
            .map_err(|_| {
                Box::new(NotificationError::LogLockPoisoned) as NotificationTransportError
            })?
            .push(notification.clone());
        Ok(())
    }
}

/// Adapter that sends `mail` notifications through the configured [`Mail`]
/// transport, allowing one production mail integration to serve both APIs.
#[derive(Clone, Copy, Default)]
pub struct MailNotificationDriver;

#[derive(Debug, Error)]
#[error("mail notification driver received {channel:?} channel")]
struct InvalidMailChannel {
    channel: NotificationChannel,
}

impl NotificationDriver for MailNotificationDriver {
    fn name(&self) -> &'static str {
        "mail"
    }

    fn send(&self, notification: &Notification) -> NotificationTransportResult<()> {
        if notification.channel != NotificationChannel::Mail {
            return Err(Box::new(InvalidMailChannel {
                channel: notification.channel.clone(),
            }));
        }
        Mail::send(
            MailMessage::new(&notification.recipient, &notification.title).text(&notification.body),
        )
        .map_err(|error| Box::new(error) as NotificationTransportError)
    }
}

pub struct NotificationJob {
    notifier: Notifier,
    notification: Notification,
}

impl NotificationJob {
    pub fn new(notifier: Notifier, notification: Notification) -> Self {
        Self {
            notifier,
            notification,
        }
    }
}

impl Job for NotificationJob {
    fn name(&self) -> &'static str {
        "notification.send"
    }

    fn handle(self: Box<Self>) -> JobFuture {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || self.notifier.send(self.notification))
                .await
                .context("notification worker join")??;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn welcome() -> Notification {
        Notification::new(NotificationChannel::Mail, "u@example.test", "Welcome", "Hi")
    }

    #[test]
    fn log_driver_keeps_development_notifications() {
        let driver = LogNotificationDriver::default();
        let notifier = Notifier::new(driver.clone());
        notifier.send(welcome()).unwrap();
        assert_eq!(driver.try_sent().unwrap().len(), 1);
    }

    struct FailingDriver;

    impl NotificationDriver for FailingDriver {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn send(&self, _notification: &Notification) -> NotificationTransportResult<()> {
            Err(Box::new(io::Error::other("provider timeout")))
        }
    }

    #[test]
    fn transport_error_retains_driver_and_source() {
        let error = Notifier::new(FailingDriver).send(welcome()).unwrap_err();
        assert!(matches!(
            error,
            NotificationError::Transport { ref driver, .. } if driver == "failing"
        ));
        assert!(format!("{error:?}").contains("provider timeout"));
    }

    #[test]
    fn channel_router_requires_an_explicit_transport() {
        let notifier = Notifier::new(NotificationRouter::new());
        let error = notifier.send(welcome()).unwrap_err();
        assert!(
            error
                .source()
                .is_some_and(|source| source.to_string().contains("no notification transport"))
        );
    }

    #[test]
    fn channel_router_delivers_to_the_selected_driver() {
        let mail = LogNotificationDriver::default();
        let notifier =
            Notifier::new(NotificationRouter::new().route(NotificationChannel::Mail, mail.clone()));
        notifier.send(welcome()).unwrap();
        assert_eq!(mail.try_sent().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn notification_runs_as_a_queue_job() {
        let driver = LogNotificationDriver::default();
        let notifier = Notifier::new(driver.clone());
        let queue = Queue::memory(1);
        notifier.dispatch(&queue, welcome()).await.unwrap();
        let (name, result) = queue.work_once().await.unwrap();
        assert_eq!(name, "notification.send");
        result.unwrap();
        assert_eq!(driver.try_sent().unwrap().len(), 1);
    }
}
