# 错误模型

Namix 将错误分为两类，避免控制器以 `String` 约定状态码或泄露底层细节。

## 面向请求的 `AppError`

业务代码返回 `Result<T, AppError>`：

- `validation`、`bad_request`、`conflict`、`Unauthenticated`、`Forbidden` 与 `NotFound` 自动映射对应 HTTP/Action JSON 响应。
- `AppError::internal(error)` 保留 `source` 错误链并写入 tracing，浏览器及 Action 只收到通用的 `500 internal server error`。
- 仅有文本时使用 `AppError::internal_message(...)`；有具体错误类型时使用 `internal(error)`，以保留诊断链。

```rust
let file = storage.put_with_policy("avatars/a.png", bytes, &policy)?;
// StorageError 自动转换为 AppError：上传策略问题是 4xx，I/O 是带 source 的 500。
```

## 框架与基础设施错误

- `thiserror`：公共库边界使用可匹配的错误枚举。当前包括 `AppError`、`ConfigError`、`StorageError`、`MailError`、`SmsError` 和 `WsError`。
- `anyhow`：队列 Job 等应用/基础设施边界使用 `anyhow::Result`，可以通过 `.context("...")` 逐层补充操作信息；worker 使用完整链路记录日志。

```rust
fn handle(self: Box<Self>) -> namix::JobFuture {
    Box::pin(async {
        send_mail().await.context("welcome mail")?;
        Ok(())
    })
}
```

库公开 API 继续返回具体错误类型；`anyhow` 不穿透 HTTP 或业务领域边界。
