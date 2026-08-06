//! Channel-neutral notifications. Development uses an inspectable log driver;
//! production supplies SMTP, HTTP API, push, or chat drivers through the trait.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    pub fn data(mut self, data: impl Serialize) -> Self {
        self.data = serde_json::to_value(data).unwrap_or(serde_json::Value::Null);
        self
    }
}
pub trait NotificationDriver: Send + Sync + 'static {
    fn send(&self, notification: &Notification) -> Result<(), String>;
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
    pub fn send(&self, notification: Notification) -> Result<(), String> {
        self.driver.send(&notification)
    }
}
#[derive(Clone, Default)]
pub struct LogNotificationDriver {
    sent: Arc<Mutex<Vec<Notification>>>,
}
impl LogNotificationDriver {
    pub fn sent(&self) -> Vec<Notification> {
        self.sent.lock().expect("notification log").clone()
    }
}
impl NotificationDriver for LogNotificationDriver {
    fn send(&self, n: &Notification) -> Result<(), String> {
        tracing::info!(channel=?n.channel,to=%n.recipient,title=%n.title,"notification sent");
        self.sent.lock().expect("notification log").push(n.clone());
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_driver_keeps_development_notifications() {
        let d = LogNotificationDriver::default();
        let n = Notifier::new(d.clone());
        n.send(Notification::new(
            NotificationChannel::Mail,
            "u@example.test",
            "Welcome",
            "Hi",
        ))
        .unwrap();
        assert_eq!(d.sent().len(), 1)
    }
}
