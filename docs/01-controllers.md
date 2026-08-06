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

    req.view("login")          // 对应 views/pages/login.tsx
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
| `.ssr()` | 资料页、文章列表、纯展示 | 服务端 HTML，**不加载**客户端 React |
| `.island()` | 登录/注册等要交互 | SSR HTML + hydrate，可用 `useForm` / `Link` |
| `.spa()`（默认） | 只要客户端挂载 | 空壳 + props key，前端再拉 props |

`view("login")` 的名字必须和 `views/pages/login.tsx`、生成注册表一致。

### 常用 Request 读取

```rust
req.query("page")                 // Option<&str>
req.query_or("redirect", "/me")   // 带默认
req.param("id")                   // 路径参数（字符串）
req.header("authorization")
req.cookie("namix_session")
req.json::<T>()                   // body JSON
req.flash()                       // 闪存（读一次后通常被 consume）
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
}

/// seal：这些字段在传输层加密（需 action_seal 开启）
#[server(name = "register", seal = ["password", "password_confirmation"])]
pub async fn register_action(req: Request) -> Result<ActionOk<AuthOk>, ActionError> {
    // 校验失败 → ValidationError → 自动变成 ActionError（字段袋）
    let form = RegisterRequest::from_values(&req)?;

    let user = match UserService::new()
        .register(&form.username, &form.password)
        .await
    {
        Ok(u) => u,
        Err(e) if e.contains("taken") => {
            return Err(ActionError::field("username", "username already taken"));
        }
        Err(e) => return Err(ActionError::message(e)),
    };

    let session_id = SessionService::new().issue(&user);

    Ok(ActionOk::new(AuthOk {
        redirect: "/me".into(),
    })
    .with_cookie(SESSION_COOKIE, session_id)
    .with_clear_cookie(LEGACY_SESSION_COOKIE))
}
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

### 成功时的 cookie

```rust
ActionOk::new(data)
    .with_cookie("namix_session", id)
    .with_clear_cookie("namix_user")   // 清旧 cookie
```

前端 `useForm` 默认会跟随返回体里的 `redirect` 做软导航。

### `#[server]` 参数形态

| 签名 | 说明 |
|------|------|
| `async fn foo(req: Request) -> ...` | 最常用 |
| `async fn foo(req: Request, body: T) -> ...` | 额外类型参数（按宏支持） |
| 返回 `Result<ActionOk<T>, ActionError>` | 推荐 |

**不要**把 `FormRequest` 当作 handler 提取器用在 `#[server]` 上——提取器失败会走 HTML 闪存跳转；Action 里应 `from_values(&req)?`。

---

## 3. 经典 POST + 闪存跳转

适合 SSR 页上的 `<form method="post">`（如个人资料、发帖）。

```rust
use crate::middleware::extract::AuthUser;
use crate::route;
use crate::validators::profile_form::ProfileRequest;

pub async fn save(req: Request, user: AuthUser, form: ProfileRequest) -> Response {
    match UserService::new()
        .save_profile(user.id, &form.display_name, &form.email, &form.bio)
        .await
    {
        Ok(_) => req.redirect_ok_to(route::main::me),
        Err(msg) => req.redirect_error_to(route::main::me, msg),
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
not_found()
no_content()
Response::redirect("/login")              // 302
Response::redirect_see_other("/login")    // 303
```

登出示例（SSR 顶栏可能无 hydrate，故用 GET）：

```rust
pub async fn logout_page(req: Request) -> Response {
    // 清会话…
    Response::redirect_see_other("/")
        .with_clear_cookie(SESSION_COOKIE)
        .with_clear_cookie(LEGACY_SESSION_COOKIE)
}
```

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
| 忘记 `camelCase` | 页面 DTO 加 `#[serde(rename_all = "camelCase")]` |