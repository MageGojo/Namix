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
let post = Post::find(id).await.or_not_found()?;
authorize(&*user, &PostPolicy, Ability::Update, Some(&post))?;

// Service：冲突 / 校验用具名构造；DB I/O 用 internal
if User::find_by_username(username).await.is_some() {
    return Err(AppError::validation("username", "username.taken"));
}
db::run(/* … */).await.map_err(AppError::internal)?;

// 出站 HTTP：对方 5xx / 超时同样 internal，不要把响应原文丢给 Action / 页面
client.get(url).send().await.map_err(AppError::internal)?;

// Storage：策略/键/只读/未知盘 → 4xx；I/O → 带 source 的 500
let storage = Storage::disk("local")?;
storage.put_with_policy("avatars/a.png", bytes, &policy)?;
```

`StorageError` 映射：`InvalidKey` / `UnknownDisk` / `Unsupported` / `InvalidJson` / 坏图 → 400；策略违规 → 422；签名过期或只读盘 → 403；对象不存在 → 404；其余 I/O/后端 → 500。

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

## 可选 HTML 错误页

这是**可选**能力。不注册时，浏览器看到框架通用 HTML；JSON 与 Action 不变。

```rust
// routes/web.rs
pub fn routes() -> Router {
    routes! { /* … */ }
        .error_page(404, errors::page)
        .error_pages(errors::page) // 403 / 500 / 429 …
}

// 控制器：走错误页
return req.not_found();
return req.forbidden();
return req.error_response(StatusCode::FORBIDDEN, "VIP only");
```

渲染器签名：`fn(&Request, ErrorPage) -> Response`。框架会把 HTTP 状态码强制成对应值。JSON 请求（`Accept: application/json` 或路径 `/api/…`）和 `#[server]` **不会**走 HTML 页。

骨架：`nx make error`。不要在渲染器里再调 `req.not_found()`（会递归）。

## 与授权 / 会话

| 场景 | 错误 |
|------|------|
| 未登录访问需登录资源 | 中间件 / 提取器 → 跳转或 `Unauthenticated` |
| Policy 拒绝 | `Forbidden` |
| JWT 坏 / 过期 | `Unauthenticated`（见 [11-jwt-crypt](./11-jwt-crypt.md)） |
| Crypt 密文损坏 | 通常 `bad_request` |
