# 邮件与短信

业务通过门面 `Mail` / `Sms` 发信；驱动与落盘由 `namix.toml` 的 `[mail]` / `[sms]` 控制。示例控制台：`GET /mailbox`（需登录）。

开关步骤总表见 [FEATURES.md](./FEATURES.md)。

---

## 1. 配置

```toml
[mail]
driver = "log"                 # 开发：日志 + 本地 outbox
from = "noreply@namix.local"
# store = "./storage/mail"     # outbox / inbox 路径（按驱动）

[sms]
driver = "log"
# store = "./storage/sms"
```

生产换成真实 SMTP / 短信网关驱动后，门面调用不变。

---

## 2. 发信 API

```rust
use namix::prelude::*;

Mail::send(
    MailMessage::new("user@example.com", "欢迎加入 Namix")
        .text(format!("你好 {}", username)),
)?;

Sms::send_code("13800000000")?;          // 或驱动提供的发送接口
Sms::verify_code("13800000000", "123456")?;
```

监听器里也可发欢迎信（见 [事件](./10-events.md)）。密码重置邮件在 `password_reset_request` Action 中发送。

---

## 3. 示例页面与 Action

| 项 | 位置 |
|----|------|
| 页面 | `GET /mailbox` → `controllers/mailbox.rs` + `views/pages/mailbox.tsx` |
| 入站 webhook | `POST /webhooks/mail/inbound`（可模拟收信） |
| Actions | `mail_send`、`mail_simulate_inbound`、`sms_send_code`、`sms_verify_code` |
| 生成 TS | `views/generated/actions/mail_*.ts`、`sms_*.ts` |

Island 页用 `useForm` / `callRust` 调 Action；成功体常带 `redirect: "/mailbox"` 刷新列表。

```ts
import { mail_send } from '../generated/actions/mail_send'
import { sms_send_code } from '../generated/actions/sms_send_code'
```

---

## 4. 入站邮件

开发可调 `mail_simulate_inbound` 或对 webhook POST，便于测「收到邮件 → 业务处理」而不接真 SMTP。生产 webhook 须校验签名/共享密钥（按接入方文档配置，勿对公网裸开）。

---

## 5. 通知骨架

```bash
nx make notification OrderShipped
nx make mail Welcome
```

`Notification` 抽象可扇出到邮件/短信/其它渠道；第一版以门面 + 示例控制台为主，复杂多渠道再拆通知类。

---

## 易错点

| 问题 | 正确做法 |
|------|----------|
| 密码重置响应泄露「用户是否存在」 | 始终返回相同形状（示例已统一 `accepted: true`） |
| 在控制器堆 SMTP 细节 | 用 `Mail::send`；密钥进环境 / 生产 toml |
| 忘记登录门 | `/mailbox` 在 `require_login` 组内 |
