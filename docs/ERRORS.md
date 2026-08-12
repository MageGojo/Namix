# 错误模型

Namix 将错误分为两类，避免控制器以 `String` 约定状态码或泄露底层细节。

## 面向请求的 `AppError`

业务代码返回 `Result<T, AppError>`：

| 构造 / 变体 | 典型 HTTP |
|-------------|-----------|
| `validation` / 字段袋 | 422 |
| `bad_request` | 400 |
| `conflict` | 409 |
| `Unauthenticated` | 401 |
| `Forbidden`（含 `authorize` 失败） | 403 |
| `NotFound` | 404 |
| `AppError::internal(error)` | 500（浏览器/Action 只见通用文案） |
| `AppError::internal_message(...)` | 500（无底层 `source` 时） |

- `internal(error)` 保留 `source` 错误链并写入 tracing。
- 仅有文本时用 `internal_message`；有具体错误类型时用 `internal(error)`。

```rust
// 控制器：Policy 失败 → 403；查无 → 404
let post = Post::find(id).await.ok_or(AppError::NotFound)?;
authorize(&*user, &PostPolicy, Ability::Update, Some(&post))?;

// Service：冲突 / 校验用具名构造；DB I/O 用 internal
if User::find_by_username(username).await.is_some() {
    return Err(AppError::conflict("username already taken"));
}
db::run(/* … */).await.map_err(AppError::internal)?;

// Storage：策略问题是 4xx，I/O 是带 source 的 500
let file = storage.put_with_policy("avatars/a.png", bytes, &policy)?;
```

经典 POST 若要把业务错误闪回页面：

```rust
Err(error) => Ok(req.redirect_error_to(route::main::posts, error.message()))
```

`#[server]` 里 `?` 会映射为 Action JSON；`ActionError::field` / `message` 供表单字段袋。

## 框架与基础设施错误

- `thiserror`：公共库边界使用可匹配的错误枚举。当前包括 `AppError`、`ConfigError`、`StorageError`、`MailError`、`SmsError`、`WsError`、`JwtError`、`CryptError`、`CacheError` 等。
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

## 与授权 / 会话

| 场景 | 错误 |
|------|------|
| 未登录访问需登录资源 | 中间件 / 提取器 → 跳转或 `Unauthenticated` |
| Policy 拒绝 | `Forbidden` |
| JWT 坏 / 过期 | `Unauthenticated`（见 [11-jwt-crypt](./11-jwt-crypt.md)） |
| Crypt 密文损坏 | 通常 `bad_request` |
