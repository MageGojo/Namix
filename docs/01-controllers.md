# 控制器

控制器是**自由异步函数**，不是类。框架把常用能力实现在 `Request` 上（`Controller` trait），业务里直接 `req.view(...)` / `req.redirect_ok_to(...)`。

入口装配见 `app/src/main.rs`：挂全局中间件 → `routes(web::routes())` → `Boot::run`。

---

## 设计原理

1. **薄控制器**：读请求、调 Service、选渲染/跳转；写库放 `services/`。
2. **两种写操作出口**
   - 可交互页（Island）：`#[server]` → JSON `ActionOk` / `ActionError`
   - 纯 SSR 表单：`Route::post` + flash 跳转
3. **页面 DTO 一次定义**：`#[derive(Serialize, ViewData)]` → 自动生成 TS 类型。
4. **认证靠提取器**：路由挂 `require_login` 后，参数写 `user: AuthUser`。

---

## 1. GET 页面：`req.view`

```rust
use namix::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct LoginPage {
    pub error: Option<String>,
    pub redirect: String,
    pub brand_icon: String,
    pub registered_count: u64,
}

pub async fn login(req: Request) -> Response {
    let registered_count = User::list().await.len() as u64;

    req.view(Page::Login)      // 对应 views/pages/login.tsx；勿手写与文件名不一致的字符串
        .island()              // 渲染模式：ssr | island | spa
        .title("登录")
        .data(LoginPage {
            error: req.flash().error,
            redirect: req.query_or("redirect", "/me").to_string(),
            brand_icon: String::new(),
            registered_count,
        })
        .render()
}
```

### 渲染模式怎么选

| 模式 | 何时用 | 行为 |
|------|--------|------|
| `.ssr_html(html)` | Rust 模板、经典表单 | 直接输出可信的服务端 HTML，不加载客户端 React |
| `.ssr()` | React 展示页、SSR 优先 | 有 Rust 正文时输出纯 HTML；正文为空时自动内联 props 并挂载，避免白屏 |
| `.island()` | 登录/注册等要交互 | 可选 SSR HTML + 内联 props；客户端 mount/hydrate，可用 `useForm` / `Link` |
| `.spa()`（默认） | 只要客户端挂载 | 空壳 + props key，前端再拉 props |

`view("login")` 的名字必须和 `views/pages/login.tsx`、生成注册表一致。推荐 `req.view(Page::Login)`（`app/src/view.rs` 由 namix-build 生成，写错页面名会编不过）。`view::login` 仍可用。

文档壳（`<html>` / `<body>` / 暗亮色）用 `Document`：`.html` / `.body` / `.head` / `.template` / `.template_file`，不依赖 class。见 [`05-frontend.md`](./05-frontend.md) §6.1。

### 常用 Request 读取

```rust
req.query("page")                 // Option<String>
req.query_or("redirect", "/me")   // 带默认
req.input("title")                // query + JSON / 表单字段（Laravel `$request->input`）
req.input_or("title", "")
req.param("id")                   // 路径参数（字符串）
req.user()                        // Option<&LoginUser>（需 `use crate::prelude::*`）
req.ip()                          // 对等 `client_ip()`
req.header("authorization")
req.bearer()                      // `Authorization: Bearer …`
req.cookie("namix_session")
req.csrf_token()                  // SSR 表单 / ViewData；中间件未跑时为空串
req.json::<T>()                   // 整段 body JSON
req.flash()                       // 闪存（读一次后通常被 consume；Crypt 自动加密封装）
```

当前用户更常见的写法是提取器（对齐 Laravel 的方法注入，未登录直接跳登录页）：

```rust
pub async fn show(req: Request, user: AuthUser) -> Response { /* … */ }
```

Sanctum 那种 `return $request->user()` 对应：

```rust
GET "/user" => |user: AuthUser| json(serde_json::json!({
    "id": user.id,
    "username": user.username,
})), name: "user", middleware = [require_login],
```

API 带 `Authorization: Bearer`（JWT）即可，不必再抄 `auth:sanctum`；浏览器走 Cookie。不要把 `session_id` / 角色原样 JSON 出去。闭包里也能 `req.user()`。

### 零授权 props（重要）

页面 **不要** 把 `userId` / `isVip` / roles / token 放进 `ViewData`。用 `AuthView` 在服务端分支，只下发展示数据：

```rust
let auth = AuthView::new(current(&req));
let (greeting, nav_links) = auth.choose(
    || ("未登录".into(), guest_nav()),
    |user| (format!("你好，{}", user.username), user_nav(user)), // VIP 链接只在这里插入
);
```

已登录禁止访问登录/注册：路由挂 `require_guest`。

写操作（更新/删除资源）不要信前端的 `user_id`：从数据库加载模型，再用 `authorize` 比对会话用户与库里的归属。详见 [授权](./07-authorization.md)。

```rust
let post = Post::find(form.post_id).await.or_not_found()?;
authorize(&*user, &PostPolicy, Ability::Update, Some(&post))?;
```

---

## 2. Server Action：`#[server]`

登录/注册写操作**不要**再在 `web.rs` 写 `POST /login`。宏会挂到统一入口 `POST /api/a`，并生成 TS：

```ts
import { login } from '../generated/actions/login'
```

### 完整业务示例（注册）

```rust
#[derive(Debug, Serialize)]
pub struct AuthOk {
    pub redirect: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
}

/// seal：这些字段在传输层加密（需 action_seal 开启）
#[server(name = "register", seal = ["password", "password_confirmation"])]
pub async fn register_action(req: Request) -> Result<ActionOk<AuthOk>, AppError> {
    // 校验失败 → ValidationError → 自动变成 AppError / ActionError（字段袋）
    let form = RegisterRequest::from_values(&req)?;
    let user = UserService::new()
        .register(&form.username, &form.password)
        .await?;

    // Cookie opaque + JWT Bearer 共用同一 sid（SessionStore）
    let tokens = SessionService::new().rotate_pair(session_id_from(&req).as_deref(), &user);

    Ok(ActionOk::new(AuthOk::with_tokens("/me", &tokens))
        .with_cookie_options(
            SESSION_COOKIE,
            tokens.cookie_token,
            SessionService::cookie_options(), // Max-Age = [session].lifetime_secs
        )
        .with_clear_cookie(LEGACY_SESSION_COOKIE))
}
```

登录/注册成功体大致为：

```json
{
  "redirect": "/me",
  "access_token": "eyJ…",
  "token_type": "Bearer",
  "expires_in": 3600
}
```

浏览器 SPA 继续跟 `redirect`；API / 移动端用 `Authorization: Bearer <access_token>`。时长见 `namix.toml`：

```toml
[session]
lifetime_secs = 604800       # Cookie / opaque
jwt_lifetime_secs = 3600     # Bearer JWT
```

### 错误 API（给前端 `useForm`）

```rust
ActionError::message("something went wrong")           // 总错误 → errors._
ActionError::field("password", "invalid username or password")
ActionError::field("username", "required").with_field("password", "required")
```

JSON 形态大致为：

```json
{ "error": "...", "message": "...", "errors": { "username": "...", "password": "..." } }
```

### 成功时的 cookie / JWT

```rust
// 推荐：带 Max-Age（Laravel 式可配置过期）
ActionOk::new(AuthOk::with_tokens("/me", &tokens))
    .with_cookie_options(
        SESSION_COOKIE,
        tokens.cookie_token,
        SessionService::cookie_options(),
    )
    .with_clear_cookie(LEGACY_SESSION_COOKIE)

// 单次自定义时长（例如「记住我」30 天）
SessionService::cookie_options_for(Duration::from_secs(60 * 60 * 24 * 30))
```

前端 `useForm` 默认会跟随返回体里的 `redirect` 做软导航。

### `#[server]` 参数形态

| 签名 | 说明 |
|------|------|
| `async fn foo(req: Request) -> ...` | 最常用 |
| `async fn foo(req: Request, body: T) -> ...` | 额外类型参数（按宏支持） |
| 返回 `Result<ActionOk<T>, ActionError>` 或 `AppError` | 推荐 |

**不要**把 `FormRequest` 当作 handler 提取器用在 `#[server]` 上——提取器失败会走 HTML 闪存跳转；Action 里应 `from_values(&req)?`。

### 在 Server Action 里调第三方

出站 HTTP 发生在 **Namix 进程**里（`reqwest` 写在 `services/`，见 [平台 · 出站 HTTP](./08-platform.md#7-出站-http-调第三方)）。浏览器看不到 API Key 和原始请求。

会回到前台的只有 `ActionOk<T>` / `ActionError`（`action_seal` 只加密传输，WASM 解密后页面 JS 仍能看到成功体）。

```rust
// 漏：把对方整包转给浏览器
Ok(ActionOk::new(raw_from_vendor))

// 不漏：只下发展示字段；密钥留在 Service
#[derive(Serialize)]
struct WeatherOk { pub summary: String }

let raw = WeatherClient::from_env()?.city("shanghai").await?;
Ok(ActionOk::new(WeatherOk {
    summary: raw["text"].as_str().unwrap_or("").into(),
}))
```

- `#[server]` **默认不挂** `require_login`。要登录才能看的数据，函数里查 `req.user()` / `current(&req)`，没有人就 `AppError::Unauthenticated`。
- 对方失败用 `AppError::internal(err)`，不要 `ActionError::message(vendor_body)`。

---

## 3. 经典 POST + 闪存跳转

适合 SSR 页上的 `<form method="post">`（如个人资料、发帖）。

```rust
use crate::prelude::*;
use crate::middleware::extract::AuthUser;
use crate::validators::profile_form::ProfileRequest;

pub async fn save(req: Request, user: AuthUser, form: ProfileRequest) -> Response {
    match UserService::new()
        .save_profile(user.id, &form.display_name, &form.email, &form.bio)
        .await
    {
        Ok(_) => req.redirect_ok_to(AppRoute::Me),
        Err(error) => req.redirect_error_to(AppRoute::Me, error.message()),
    }
}
```

| 助手 | 行为 |
|------|------|
| `req.redirect_ok_to(route)` | 303 + flash ok |
| `req.redirect_error_to(route, msg)` | 303 + flash error |
| `req.see_other_to(route)` | 303，无 flash |
| `req.redirect_guest_to(route)` | 未登录访客跳转（可带 redirect 回跳） |
| `req.redirect_back()` | 回上一页 |

GET 页用 `req.flash().error` / `.ok` 展示消息（`view` 渲染会 consume flash）。

---

## 4. 提取器：`AuthUser`、`Path`

```rust
// middleware/extract.rs
pub struct AuthUser(pub LoginUser);  // Deref → LoginUser

pub async fn show(req: Request, user: AuthUser) -> Response {
    let username = user.username.clone();
    // ...
}

pub async fn profile(req: Request, Path(id): Path<u64>, user: AuthUser) -> Response {
    // /profile/:id
}
```

- `AuthUser` 依赖全局 `hydrate` 把会话放进 `req`，且路由应挂 `require_login`。
- 未登录：`FromRequest` 失败 → `redirect_guest_to(login)`。

路径参数、JSON 等也走 `FromRequest`：

```rust
Path<u64>
Json<MyBody>
```

---

## 5. 其它常用响应

```rust
use namix::prelude::*;

text("ok")
html("<h1>hi</h1>")
json(&serde_json::json!({ "ok": true }))
json_raw(r#"{"ok":true}"#)   // 已是 JSON 字符串时用，避免二次编码
not_found()                  // 纯文本 404，不含自定义页
req.not_found()              // HTML 优先走自定义错误页，否则与 AppError::NotFound 相同
req.forbidden()
req.error_response(namix::http::StatusCode::FORBIDDEN, "VIP only")
no_content()
Response::redirect("/login")              // 302
Response::redirect_see_other("/login")    // 303
```

登出用 **POST**（经 Origin + CSRF）。SSR 顶栏无 hydrate 时，用经典 `<form method="post">` + `<CsrfField />`，不要改成 GET：

```rust
/// 经典表单退出；POST 会经过 Origin + CSRF 校验。
pub async fn logout_page(req: Request) -> Response {
    if let Some(sid) = session_id_from(&req) {
        if let Err(error) = SessionService::new().revoke(&sid) {
            return error.into_response_for(&req);
        }
    }
    Response::redirect_see_other("/")
        .with_clear_cookie_options(SESSION_COOKIE, SessionService::cookie_options())
        .with_clear_cookie(LEGACY_SESSION_COOKIE)
}
```

Island / Action 侧也可调用生成的 `logout`：

```rust
#[server(name = "logout")]
pub async fn logout_action(req: Request) -> Result<ActionOk<AuthOk>, AppError> {
    if let Some(sid) = session_id_from(&req) {
        SessionService::new().revoke(&sid)?;
    }
    Ok(ActionOk::new(AuthOk::redirect_only("/"))
        .with_clear_cookie_options(SESSION_COOKIE, SessionService::cookie_options())
        .with_clear_cookie(LEGACY_SESSION_COOKIE))
}
```

### 全设备登出与密码重置

```rust
/// 结束当前用户全部设备会话（含本机）。
#[server(name = "logout_all")]
pub async fn logout_all_action(req: Request) -> Result<ActionOk<AuthOk>, AppError> {
    let user = req.get::<LoginUser>().ok_or(AppError::Unauthenticated)?;
    SessionService::new().revoke_all_for_user(user.id)?;
    Ok(ActionOk::new(AuthOk::redirect_only("/"))
        .with_clear_cookie_options(SESSION_COOKIE, SessionService::cookie_options())
        .with_clear_cookie(LEGACY_SESSION_COOKIE))
}

/// 申请重置：无论账号是否存在，响应形状相同，避免枚举用户。
#[server(name = "password_reset_request", seal = ["username"])]
pub async fn password_reset_request_action(
    req: Request,
) -> Result<ActionOk<PasswordResetOk>, AppError> { /* … */ }

/// 消费一次性 token（默认 30 分钟）→ 改密 → revoke_all → 跳转登录。
#[server(name = "password_reset_confirm", seal = ["token", "password"])]
pub async fn password_reset_confirm_action(
    req: Request,
) -> Result<ActionOk<AuthOk>, AppError> { /* … */ }
```

对照实现：`app/src/controllers/auth.rs`、`services/password_reset.rs`；前端 `generated/actions/logout_all.ts` 等。JWT / 密封细节见 [JWT 与 Crypt](./11-jwt-crypt.md)。

写路径授权示例（真实代码在 `controllers/posts.rs` + `policies/post_policy.rs`）：

```rust
pub async fn update(req: Request, user: AuthUser, form: PostRequest) -> Result<Response, AppError> {
    let id = req.param("id").and_then(|s| s.parse().ok()).ok_or(AppError::NotFound)?;
    let post = Post::find(id).await.or_not_found()?;
    authorize(&*user, &PostPolicy, Ability::Update, Some(&post))?;
    UserService::new().update_post(post.id, &form.title, &form.body).await?;
    Ok(req.see_other_to(AppRoute::Posts))
}
```

错误边界见 [错误模型](./ERRORS.md)。自定义 HTML 404/403 见 [路由 · 错误页](./02-routes.md#8-可选-html-错误页)。

---

## 6. 新建控制器清单

1. `nx make controller Posts`（或手写 `controllers/foo.rs`）
2. 在 `routes/web.rs` 注册 GET/POST
3. 若有页面：加 `views/pages/foo.tsx`，DTO 加 `ViewData`
4. 写操作用 `#[server]` 或经典 `FormRequest` POST
5. 重逻辑放 `services/`

自动 `mod` 由 namix-build 维护；**路由不会自动生成**，必须写 `web.rs`。

---

## 易错点

| 问题 | 正确做法 |
|------|----------|
| 给 `#[server]` 又写了 `POST /login` | 删掉，只保留 GET 页面 |
| 软导航拿到带引号的 props | 框架侧用 `json_raw`（业务勿对 page props 再 `json(String)`） |
| SSR 页里调 `useForm` | 改成 `.island()`，或改用经典 form POST |
| 在控制器里堆 SQL | 抽到 `UserService` 等 |
| `#[server]` 原样 `return` 第三方 JSON | 映射成展示 DTO；Key 留在 Service |
| 忘记 `camelCase` | 页面 DTO 加 `#[serde(rename_all = "camelCase")]` |
