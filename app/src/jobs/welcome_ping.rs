use namix::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct WelcomePing {
    pub username: String,
    pub email: String,
}

impl QueuedJob for WelcomePing {
    const NAME: &'static str = "welcome_ping";

    fn handle(self) -> JobFuture {
        Box::pin(async move {
            if !self.email.trim().is_empty() {
                Mail::send(
                    MailMessage::new(&self.email, "Namix queued hello").text(format!(
                        "这是一条延迟队列邮件，重启 worker 后仍会发出。你好 {}。",
                        self.username
                    )),
                )?;
            }
            namix::log::info!("welcome_ping delivered for {}", self.username);
            Ok(())
        })
    }
}

pub fn register() {
    register_job::<WelcomePing>();
}
