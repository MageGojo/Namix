# Namix 业务开发文档

面向在 `app/` 里写业务的指南：讲清楚**怎么用**和**为什么这样设计**，不展开框架内部实现细节。

| 文档 | 内容 |
|------|------|
| [控制器](./01-controllers.md) | Request / Response、页面渲染、`#[server]`、提取器、闪存跳转 |
| [路由](./02-routes.md) | `web.rs` 注册、命名路由、中间件、Boot 自动挂载 |
| [验证器](./03-validators.md) | FormField / FormRequest、规则、两种失败出口 |
| [模型](./04-models.md) | Toasty Model、关联、`db` 助手、Service / Seeder |
| [前端交互](./05-frontend.md) | 渲染模式、`useForm` / `Link` / `callRust`、生成产物 |
| [SSE / WebSocket](./06-realtime.md) | `Sse` 推流、`Route::ws`、WSS |
| 邮件 / 短信 | `Mail` / `Sms` 门面（`namix.toml` `[mail]`/`[sms]`）；页面 `/mailbox`；入站 `POST /webhooks/mail/inbound` |

## 一条请求怎么走

```text
浏览器
  ├─ GET 页面 ──► Route::get → 控制器 req.view(...).render()
  │                 ├─ spa / island：可 hydrate + 软导航
  │                 └─ ssr：纯 HTML（无客户端 JS）
  │
  ├─ 经典表单 POST ──► Route::post + FormRequest 提取器
  │                      └─ redirect_ok_to / redirect_error_to + flash
  │
  └─ Server Action ──► useForm → generated/actions → POST /api/a
                         └─ #[server] + FormRequest::from_values
                              └─ ActionOk{redirect}+cookie | ActionError{errors}
```

## 目录约定（业务侧）

```text
app/src/
  controllers/     # 处理器（自由函数，不是类）
  routes/web.rs    # 页面与经典 POST 路由表
  validators/      # 表单校验
  models/          # Toasty 实体 + 简洁查询助手
  services/        # 写库、会话等业务逻辑（控制器宜薄）
  middleware/      # hydrate / require_login / AuthUser
  views/pages/     # React 页面（与 view 名对应）
  views/namix.ts   # 前端公共 API 出口
  views/generated/ # 自动生成：勿手改
```

## 快速对照（Laravel 习惯）

| Laravel | Namix |
|---------|--------|
| Controller 方法 | `async fn` + `Request` 上的 Controller 助手 |
| `return view('login', $data)` | `req.view("login").data(...).island().render()` |
| Form Request | `impl FormRequest` + 提取器或 `from_values` |
| Eloquent Model | `#[derive(toasty::Model)]` + `User::find` / `load_posts` |
| `route('me')` | Rust `route::main::me` / TS `route.me()` |
| Inertia `useForm` / `Link` | `import { useForm, Link } from '../namix'` |
| Livewire / RPC 写操作 | `#[server]` + `generated/actions/*` |