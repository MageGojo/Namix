# Namix 业务开发文档

面向在 `app/` 里写业务的指南：讲清楚**怎么用**和**为什么这样设计**，不展开框架内部实现细节。

**新来的先看 [五分钟上手](./START.md)**（路由名 → `AppRoute` / `Page` / `route.login()`）。  
下一阶段（Rust ↔ React 写体验，不堆功能）见 [NEXT.md](./NEXT.md) DX 节。

| 文档 | 内容 |
|------|------|
| [Features 开关](./FEATURES.md) | `nx new` lean 默认；`[features]` / Cargo / database / session / mail / sms 全表与打开步骤 |
| [控制器](./01-controllers.md) | Request / Response、页面渲染、`#[server]`、提取器、闪存跳转、登出 / 密码重置、零授权 props |
| [路由](./02-routes.md) | `web.rs` 注册、命名路由、`resource`、中间件、Boot 自动挂载 |
| [验证器](./03-validators.md) | FormField / FormRequest、规则、两种失败出口 |
| [模型](./04-models.md) | Toasty Model、关联、`db` 助手、Service / Seeder、`AppError` |
| [前端交互](./05-frontend.md) | 渲染模式、`useForm` / `Link` / `CsrfField`、生成产物 |
| [SSE / WebSocket](./06-realtime.md) | `Sse` 推流、`Route::ws`、WSS、聊天室鉴权 |
| [授权](./07-authorization.md) | Policy / Gate + 示例 `PostPolicy`（≈ Laravel authorize） |
| [平台能力](./08-platform.md) | 分页、Cache、Queue、Storage、TestClient、出站 HTTP（`reqwest`）、`nx make` |
| [邮件 / 短信](./09-mail-sms.md) | `Mail` / `Sms` 门面、`/mailbox`、webhook |
| [事件 / 监听器](./10-events.md) | `dispatch` / `listen`、注册与登录副作用 |
| [JWT 与 Crypt](./11-jwt-crypt.md) | Cookie + Bearer、HS256、AES-GCM 密封 |
| [错误模型](./ERRORS.md) | `AppError`、thiserror / anyhow 边界 |
| [SSR（Rust）](./SSR-RUST.md) | 运行时无 Node；Rust 正文 / Island 回退 |
| [生产发布](./PRODUCTION.md) | `dist/`、滚动更新、共享会话、可信代理 |
| [安全范围](./SECURITY_SCOPE.md) | CSRF、限流、Cookie/JWT、Crypt 检查清单 |
| [项目结构](./PROJECT.md) | 架构、生成链、目录职责 |

元文档（进度 / 决策 / 路线图）：[PROGRESS](./PROGRESS.md) · [DECISIONS](./DECISIONS.md) · [NEXT](./NEXT.md)。

## 一条请求怎么走

```text
浏览器
  ├─ GET 页面 ──► Route::get → 控制器 req.view(...).render()
  │                 ├─ spa / island：可 hydrate + 软导航（island 壳与 props 由 **Rust** 内联，不依赖 Node）
  │                 └─ ssr：Rust 正文优先；无正文时以内联 Island 防止空白页（详见 docs/SSR-RUST.md）
  │
  ├─ 经典表单 POST ──► Route::post + FormRequest 提取器 + CSRF(_csrf)
  │                      └─ redirect_ok_to / redirect_error_to + flash
  │                      └─ 写资源：查库 + authorize(Policy)
  │
  └─ Server Action ──► useForm → generated/actions → POST /api/a
                         └─ #[server] + FormRequest::from_values
                              └─ ActionOk{redirect, access_token?}+Set-Cookie(CookieOptions)
                                 | ActionError{errors}
```

登录/注册会同时写 Cookie（opaque，`Max-Age = [session].lifetime_secs`）并返回可选 JWT
（`access_token` / `Bearer`，时长 `jwt_lifetime_secs`）。浏览器跟 `redirect`；API 用
`Authorization: Bearer`。两者共用 Session Store 的 `sid`，登出可同时作废。详见 [JWT 与 Crypt](./11-jwt-crypt.md)。

## 目录约定（业务侧）

`nx new` lean 默认只有控制器、路由、中间件与 views；下表含可选目录，打开方式见 [FEATURES.md](./FEATURES.md)。

```text
app/src/
  controllers/     # 始终：处理器（自由函数）
  routes/web.rs    # 始终：页面与经典 POST 路由表
  middleware/      # 始终：请求中间件
  policies/        # 可选目录：存在即编译（nx make policy）
  views/pages/     # [features].pages：React 页面
  views/namix.ts   # 前端公共 API 出口
  views/generated/ # 自动生成：勿手改
  validators/      # [features].validators
  models/          # [features].models + [database]
  services/        # [features].services
  events/          # [features].events
  listeners/       # [features].listeners
  seeders/         # [features].seeders
  jobs/            # nx make job；nx work 消费
  requests/        # [features].requests（可选）
```

## 快速对照（Laravel 习惯）

| Laravel | Namix |
|---------|--------|
| `[UserController::class, 'index']` | `users::index` 自由函数；CRUD 用 `resource("users", Ctrl)` |
| `$request->user()` / `auth:sanctum` | `user: AuthUser` 或 `req.user()`；`hydrate` + `require_login`；API 用 Bearer JWT |
| `$request->input('title')` | `req.input("title")` / `req.input_or` |
| `Route::get('/hi', fn () => 'Hello')` | `GET "/greeting" => \|\| "Hello World"` / `Route::get("/greeting", \|\| "Hello World")` |
| `Http::get('https://…')` | 业务包 `reqwest`，写在 `services/`；无框架门面。见 [08 §7](./08-platform.md#7-出站-http-调第三方) |
| Broadcasting / Echo | SSE / `Route::ws`；进程内事件是 `dispatch` / `listen` |
| `return view('login', $data)` | `req.view(Page::Login).data(...).island().render()` |
| Form Request | `impl FormRequest` + 提取器或 `from_values` |
| Eloquent Model | `#[derive(toasty::Model)]` + `User::find` / `load_posts` |
| `route('me')` | Rust `AppRoute::Me` / TS `route.me()` |
| `errors/404.blade.php` | 可选 `.error_page(404, …)` / `.error_pages(…)` |
| Inertia `useForm` / `Link` | `import { useForm, Link, CsrfField } from '../namix'` |
| Livewire / RPC 写操作 | `#[server]` + `generated/actions/*` |
| Cookie 会话 / Sanctum token | opaque Cookie + 可选 HS256 JWT Bearer（共用 `sid` / Store） |
| `Cookie::make(..., $minutes)` | `SessionService::cookie_options()` / `cookie_options_for(ttl)` |
| `$this->authorize('update', $post)` | `authorize(&user, &PostPolicy, Ability::Update, Some(&post_from_db))?` |
| `PostPolicy` / Gate | `Policy` + `Gate` + `nx make policy` |
| `Mail::` / `Notification` | `Mail` / `Sms` 门面；见 [09](./09-mail-sms.md) |
| `Event::dispatch` / Listener | `dispatch` / `listen`；见 [10](./10-events.md) |
| `Crypt::encrypt` | `namix::Crypt`；见 [11](./11-jwt-crypt.md) |
