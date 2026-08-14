# 路由

业务路由集中在 `app/src/routes/web.rs`。`name: "login"` 会生成三份同一契约：

- Rust 枚举：`AppRoute::Login`（`app/src/route.rs`，编译期）
- Rust 别名：`route::main::login`（与枚举同一值）
- TypeScript：`route.login()` / `AppRoute.Login`（`views/routes.ts`：启动或 `nx export routes` 时写出，与上条时间线不一致）

不要把枚举命名成 `Route`：`Route` 已经是 `Route::get` 构建器。入门步骤见 [START.md](./START.md)。

---

## 设计原理

1. **显式注册**：页面 URL 不会因为有了 `login.tsx` 就自动出现，必须在 `web.rs` 挂上。
2. **命名是契约**：后端 `redirect_to`、前端 `route.xxx()`、软导航都靠同一个名字。
3. **写操作分流**：交互式写 → `#[server]`（不必出现在 `web.rs`）；SSR 表单写 → `Route::post`。
4. **中间件按路由挂**：登录门、VIP 门挂在具体路由上，而不是全局一刀切（会话水合除外）。

---

## 1. 基本写法

```rust
use namix::prelude::*;

use crate::controllers::{auth, home, me, posts};
use crate::middleware::auth::{require_guest, require_login, require_vip};

pub fn routes() -> Router {
    routes! {
        "/" => {
            GET "/" => home::index, name: "home",
            GET "/login" => auth::login, name: "login",
                middleware = [require_guest],
            GET "/register" => auth::register, name: "register",
                middleware = [require_guest],
            POST "/logout" => auth::logout_page, name: "logout",
            GET "/vip" => home::vip_lounge, name: "vip",
                middleware = [require_login, require_vip],
        },
        "/" => {
            GET "/me" => me::show, name: "me",
            POST "/me" => me::save, name: "me.submit",
            GET "/posts" => posts::index, name: "posts",
            POST "/posts" => posts::create, name: "posts.submit",
            POST "/posts/:id" => posts::update, name: "posts.update",
            POST "/posts/:id/delete" => posts::destroy, name: "posts.destroy",
            middleware: [require_login],
        },
    }
}
```

示例应用的完整表见 `app/src/routes/web.rs`。经典表单写操作用 **POST**（HTML 不便发 DELETE）；资源归属校验见 [授权](./07-authorization.md)。

`routes!` 是业务侧的默认写法：同前缀的路由放一个组中，`middleware:` 对整组生效；单路由中间件写在该路由后，用 `middleware = [...]`。组中间件按源码顺序执行。

支持 `GET` / `POST` / `PUT` / `PATCH` / `DELETE` / `WS`。WebSocket 处理器可接收 `WsSocket` 或 `(Request, WsSocket)`，不使用 HTTP 中间件。

需要动态组装路由时，仍可使用链式 API：

```text
Route::get(path, handler)
  .middleware(mw)     // 可多次
  .name("me.submit")  // 可选但强烈建议
  .register()         // 收成 Router，再 .merge
```

支持的方法：`get` / `post` / `put` / `patch` / `delete`（以框架 `Route` API 为准）。

### 闭包路由

短响应不必单独写控制器。同步闭包即可（对齐 Laravel 的 `Route::get('/greeting', fn () => 'Hello World')`）：

```rust
Route::get("/greeting", || "Hello World")
    .name("greeting")
    .register()

// 或写进 routes!
GET "/greeting" => || "Hello World", name: "greeting",
GET "/hi/:name" => |req: Request| format!("Hello {}", req.param_or("name", "world")),
    name: "hi",
```

返回 `&'static str` / `String` / `Response` / `Result<_, AppError>`。读库、渲染页面仍用 `async fn` 控制器。

PHP 的 `[UserController::class, 'index']` 在 Rust 里就是模块函数：`GET "/users" => users::index`。CRUD 七件套用 `resource("users", UsersController)`（见下 §4）。不要再造一份 Class + 方法名数组。

---

## 2. 命名规则

| `name:` | Rust | TypeScript |
|---------|------|------------|
| `"home"` | `AppRoute::Home` / `route::main::home` | `route.home()` / `AppRoute.Home` |
| `"login"` | `AppRoute::Login` | `route.login()` / `AppRoute.Login` |
| `"me.submit"` | `AppRoute::MeSubmit` / `route::main::me_submit` | `route.me.submit()` / `AppRoute.MeSubmit` |
| `"posts"` | `AppRoute::Posts` | `route.posts()` / `AppRoute.Posts` |

点号、下划线都会收成 PascalCase。业务文件用 `use crate::prelude::*` 即可拿到 `AppRoute` 与 `Page`。

带路径参数：

```rust
GET "/profile/:id" => profile::show, name: "profile",
```

```rust
AppRoute::Profile.to(&[("id", "5")])  // → Some("/profile/5")
AppRoute::Home.href()                 // 无参路径
```

漏参数会得到 `None` 或拼出仍带 `:id` 的路径。计划收成 `AppRoute::profile("5")` 这类类型化参数，见 [NEXT.md](./NEXT.md) DX 节。

```ts
import { route, AppRoute } from '../namix'

route.profile({ id: 5 })
route(AppRoute.Profile, { id: 5 })
```

### 在控制器里用命名路由

```rust
use crate::prelude::*;

req.redirect_ok_to(AppRoute::Me)
req.redirect_error_to(AppRoute::Posts, "title required")
req.redirect_guest_to(AppRoute::Login)
req.redirect_to(AppRoute::Home)
```

`route::main::me` 仍可用，和 `AppRoute::Me` 是同一个枚举值。

---

## 3. 中间件

### 全局（`main.rs`）

```rust
.middleware(app::middleware::logger::access_log)
.middleware(app::middleware::session::hydrate)  // Cookie 或 Bearer(opaque|JWT) → req.set::<LoginUser>
.routes(app::routes::web::routes())
```

### 按路由

```rust
.middleware(require_login)   // 无会话 → 去登录页
.middleware(require_guest)   // 已登录 → 离开登录/注册页
.middleware(require_vip)     // 非 VIP → 拒绝/跳转（见 auth 中间件实现）
```

顺序：先写的先执行。VIP 路由应先 `require_login` 再 `require_vip`。

认证提取器 `AuthUser` 依赖 `hydrate` 已写入 `LoginUser`；仅挂提取器、不挂 `require_login` 时，未登录会在 `FromRequest` 里被重定向。

对具体资源的「能不能改这条」不靠中间件猜 body，而用 Policy：查库后 `authorize`（见 [授权](./07-authorization.md)）。

---

## 4. 资源路由 `resource`

Laravel 风格七件套（`index` / `create` / `store` / `show` / `edit` / `update` / `destroy`）。未实现的动作默认 `405`。

```rust
use namix::prelude::*;

#[derive(Clone)]
struct PostsController;

impl ResourceController for PostsController {
    fn index(&self, _req: Request) -> ResourceFuture<'_> {
        Box::pin(async { Ok(text("posts.index")) })
    }
    fn store(&self, req: Request) -> ResourceFuture<'_> {
        Box::pin(async move {
            // 提取器 / authorize / Service…
            Ok(Response::redirect_see_other("/posts"))
        })
    }
    fn update(&self, req: Request) -> ResourceFuture<'_> {
        Box::pin(async move {
            let _id = req.param_or("id", "");
            Ok(Response::redirect_see_other("/posts"))
        })
    }
    fn destroy(&self, req: Request) -> ResourceFuture<'_> {
        Box::pin(async move {
            let _id = req.param_or("id", "");
            Ok(Response::redirect_see_other("/posts"))
        })
    }
}

// 在 routes() 里：
// router.merge(resource("posts", PostsController))
```

| 方法 | 路径 | 路由名 |
|------|------|--------|
| GET | `/posts` | `posts.index` |
| GET | `/posts/create` | `posts.create` |
| POST | `/posts` | `posts.store` |
| GET | `/posts/:id` | `posts.show` |
| GET | `/posts/:id/edit` | `posts.edit` |
| PATCH | `/posts/:id` | `posts.update` |
| DELETE | `/posts/:id` | `posts.destroy` |

骨架：`nx make resource Posts`。  
仓库示例 `posts` **手写**了 `GET/POST /posts` 与 `POST /posts/:id`、`POST /posts/:id/delete`（便于 SSR 表单 + CSRF），API 形态可再改用 `resource` + PATCH/DELETE。

---

## 5. Boot 自动挂载（你不用写）

`Boot::run` 会在业务 `web::routes()` 之外合并：

| 路径 | 作用 |
|------|------|
| `GET /__namix/health` | 发布探针（状态、版本、revision） |
| `GET /__namix/routes` | 命名路由 JSON（调试 / 前端） |
| `POST /api/a` | 所有 `#[server]` 的统一入口 |
| `GET /__namix/props/:key` | SPA 拉 props |
| `GET /build/*` | 生产静态资源（始终注册） |
| `GET {prefix}/build/*` | 可选：`NAMIX_ASSET_PREFIX` 非空时的公共 URL 别名（与 HTML 标签一致） |

因此：**登录 Action 不会出现在 `web.rs`，但仍可被调用。**

---

## 6. 前端如何用路由

```ts
import { route } from '../routes'
import { Link } from '../namix'

<Link href={route.home()} prefetch>首页</Link>
<Link href={route.register()}>注册</Link>

<form method="post" action={route.me.submit()}>
  …
</form>
```

原则：**路由不进页面 props**。永远 `import { route }`，不要把 URI 塞进 ViewData。`Link` 目前只收 `href: string`，所以写成 `<Link href={route.login()}>`。

`routes.ts` 在服务启动（或 `nx export routes`）时由目录 catalog 重写，文件头有 `@generated`——**不要手改**。Rust 侧 `AppRoute` 则在 `cargo build` 时已生成，两边时间线不一致；编译期写出 TS 路由是 [NEXT.md](./NEXT.md) 的第一刀。

---

## 7. 新增一条业务路由的步骤

1. 写好 `controllers/xxx.rs` 里的 `async fn`
2. 在 `web.rs` 的 `routes!` 表中注册 `METHOD "path" => controller, name: "…"`
3. 若需登录：放入 `middleware: [require_login]` 路由组，或在单路由加 `middleware = [require_login]`
4. 编译/启动一次：Rust `AppRoute::…` 在 cargo 后出现；TS `route.xxx()` 要等启动或 `nx export routes`
5. 页面或表单里用命名路由，避免硬编码路径

---

## 8. 可选：HTML 错误页

不注册时，浏览器未匹配路由和 `AppError` 仍是框架通用 HTML；JSON / `#[server]` 始终 `{ error, message, errors }`。

```rust
pub fn routes() -> Router {
    routes! { /* 业务路由 */ }
        .error_page(404, errors::page)     // 只覆盖 404
        .error_pages(errors::page)         // 403 / 500 / 429 … 共用；具体状态优先
}
```

控制器里用 `req.not_found()` / `req.forbidden()` / `req.error_response(status, msg)`，不要用自由函数 `not_found()`（那是纯文本，看不到自定义页）。

骨架：`nx make error` 或 `nx make error 404`。示例见 `app/src/controllers/errors.rs`。

也可以写在 `Boot::error_page`；两边都写时 **`web.rs` 优先**（`TestClient` 也只看路由表）。

---

## 易错点

| 问题 | 正确做法 |
|------|----------|
| 只有 tsx 没有路由 | 必须在 `web.rs` 注册 |
| 登录 POST 与 `#[server]` 重复 | `web.rs` 只留 GET |
| 名字写了但不 `.name()` | 生成表没有该项，`redirect_to` / TS 都找不到 |
| `me.submit` 在 TS 当扁平函数 | 用 `route.me.submit()` 嵌套调用 |
| 硬编码 `"/me"` | 优先 `route.me()` / `AppRoute::Me`（redirect 字符串有时例外，如 Action 返回给前端） |
| 自定义了 404 仍看到纯文本 | 用 `req.not_found()`，并在 `routes()` 上 `.error_page` / `.error_pages` |
