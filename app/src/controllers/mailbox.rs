//! 邮箱 / 短信控制台：发信、入站 webhook、手机验证码。

use namix::http::StatusCode;
use namix::prelude::*;
use namix::server_fn::expand_input_map;
use serde::{Deserialize, Serialize};

use crate::middleware::extract::AuthUser;
use crate::view;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct MailItem {
    pub id: String,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub text: String,
    pub at: u64,
    pub direction: String,
}

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct SmsItem {
    pub id: String,
    pub to: String,
    pub body: String,
    pub at: u64,
}

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct MailboxPage {
    pub title: String,
    pub username: String,
    pub mail_from: String,
    pub mail_driver: String,
    pub sms_driver: String,
    pub outbox: Vec<MailItem>,
    pub inbox: Vec<MailItem>,
    pub sms_sent: Vec<SmsItem>,
}

#[derive(Debug, Serialize)]
pub struct MailboxOk {
    pub redirect: String,
}

/// GET /mailbox
pub async fn page(req: Request, user: AuthUser) -> Response {
    req.view(view::mailbox)
        .island()
        .title("邮箱与短信")
        .data(MailboxPage {
            title: "邮箱与短信".into(),
            username: user.username.clone(),
            mail_from: Mail::from_address(),
            mail_driver: Mail::driver(),
            sms_driver: Sms::driver(),
            outbox: Mail::outbox()
                .into_iter()
                .rev()
                .take(50)
                .map(mail_item)
                .collect(),
            inbox: Mail::inbox()
                .into_iter()
                .rev()
                .take(50)
                .map(mail_item)
                .collect(),
            sms_sent: Sms::sent()
                .into_iter()
                .rev()
                .take(50)
                .map(sms_item)
                .collect(),
        })
        .render()
}

/// POST /webhooks/mail/inbound — 模拟 ESP / IMAP 推送入站邮件。
pub async fn inbound_webhook(req: Request) -> Response {
    #[derive(Deserialize)]
    struct Body {
        from: String,
        #[serde(default)]
        to: Option<String>,
        subject: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        html: String,
    }

    let body: Body = match serde_json::from_slice(req.body()) {
        Ok(b) => b,
        Err(e) => {
            return json(serde_json::json!({ "ok": false, "error": e.to_string() }))
                .with_status(StatusCode::BAD_REQUEST);
        }
    };

    let to = body.to.unwrap_or_else(Mail::from_address);
    let msg = MailMessage::new(to, body.subject)
        .from_addr(body.from)
        .text(body.text)
        .html(body.html);

    match Mail::receive(msg) {
        Ok(()) => json(serde_json::json!({ "ok": true })),
        Err(e) => {
            json(serde_json::json!({ "ok": false, "error": e.to_string() })).with_status(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[server(name = "mail_send", seal = ["to", "subject", "text"])]
pub async fn send_mail_action(req: Request) -> Result<ActionOk<MailboxOk>, ActionError> {
    require_login(&req)?;
    let map = expand_input_map(&req).map_err(|error| ActionError::message(error.to_string()))?;

    let to = map
        .get("to")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ActionError::field("to", "to.required"))?
        .to_string();
    if !to.contains('@') {
        return Err(ActionError::field("to", "to.email"));
    }
    let subject = map
        .get("subject")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("Namix mail")
        .to_string();
    let text = map.get("text").cloned().unwrap_or_default();

    Mail::send(MailMessage::new(to, subject).text(text)).map_err(|error| ActionError::message(error.to_string()))?;

    Ok(ActionOk::new(MailboxOk {
        redirect: "/mailbox".into(),
    }))
}

#[server(name = "mail_simulate_inbound", seal = ["from", "subject", "text"])]
pub async fn simulate_inbound_action(req: Request) -> Result<ActionOk<MailboxOk>, ActionError> {
    require_login(&req)?;
    let map = expand_input_map(&req).map_err(|error| ActionError::message(error.to_string()))?;

    let from = map
        .get("from")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("friend@example.com")
        .to_string();
    let subject = map
        .get("subject")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("你好，这是一封入站测试邮件")
        .to_string();
    let text = map
        .get("text")
        .cloned()
        .unwrap_or_else(|| "模拟 IMAP / webhook 收取成功。".into());

    Mail::receive(
        MailMessage::new(Mail::from_address(), subject)
            .from_addr(from)
            .text(text),
    )
    .map_err(|error| ActionError::message(error.to_string()))?;

    Ok(ActionOk::new(MailboxOk {
        redirect: "/mailbox".into(),
    }))
}

#[server(name = "sms_send_code", seal = ["phone"])]
pub async fn send_code_action(req: Request) -> Result<ActionOk<MailboxOk>, ActionError> {
    require_login(&req)?;
    let map = expand_input_map(&req).map_err(|error| ActionError::message(error.to_string()))?;
    let phone = map
        .get("phone")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ActionError::field("phone", "phone.required"))?
        .to_string();

    Sms::send_code(&phone).map_err(|error| match error {
        SmsError::InvalidPhone => ActionError::field("phone", "phone.invalid"),
        other => ActionError::message(other.to_string()),
    })?;

    Ok(ActionOk::new(MailboxOk {
        redirect: "/mailbox".into(),
    }))
}

#[server(name = "sms_verify_code", seal = ["phone", "code"])]
pub async fn verify_code_action(req: Request) -> Result<ActionOk<MailboxOk>, ActionError> {
    require_login(&req)?;
    let map = expand_input_map(&req).map_err(|error| ActionError::message(error.to_string()))?;
    let phone = map
        .get("phone")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ActionError::field("phone", "phone.required"))?
        .to_string();
    let code = map
        .get("code")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ActionError::field("code", "code.required"))?
        .to_string();

    let ok = Sms::verify_code(&phone, &code).map_err(|error| ActionError::message(error.to_string()))?;
    if !ok {
        return Err(ActionError::field("code", "code.otp"));
    }

    Ok(ActionOk::new(MailboxOk {
        redirect: "/mailbox".into(),
    }))
}

fn require_login(req: &Request) -> Result<(), ActionError> {
    req.get::<crate::services::session::LoginUser>()
        .cloned()
        .map(|_| ())
        .ok_or_else(|| ActionError::message("unauthenticated"))
}

fn mail_item(m: MailMessage) -> MailItem {
    MailItem {
        id: m.id,
        from: m.from,
        to: m.to,
        subject: m.subject,
        text: m.text,
        at: m.at,
        direction: m.direction,
    }
}

fn sms_item(m: SmsMessage) -> SmsItem {
    SmsItem {
        id: m.id,
        to: m.to,
        body: m.body,
        at: m.at,
    }
}
