---
name: namix
description: >-
  Builds and extends Namix apps (Rust + React full-stack, nx CLI, Laravel-style
  routes/controllers, Toasty, Island/SSR, #[server] actions). Use when working
  in the Namix repo or any Namix app, writing controllers/routes/validators/
  models/pages, or when the user mentions Namix, nx new, nx make, req.view,
  #[server], FormRequest, PostPolicy, useForm, CsrfField, or Laravel-like Rust
  web apps. Never treat Namix as a separate SPA + REST API split.
---

# Namix Framework Skill

Namix = **一体全栈**（Rust 服务端 + 同仓 React 页面），不是前后端分离。

- 浏览器打开的页面由 **Rust 控制器** `req.view(...).render()` 产出
- 交互写操作用 **`#[server]` → `POST /api/a`** 或 **经典 `POST` + FormRequest + CSRF**
- TypeScript 在 `app/src/views/`，契约由 Rust 生成；**不要**另起 `frontend/` SPA 或独立 API 服务

教学文档（source of truth）：`docs/README.md`（01–11 + FEATURES / ERRORS / SSR-RUST）。

## When this skill applies

**本仓库默认**：任何 Namix / `nx` / `app/src/views` 相关编码，先读本 skill。

也适用于：新功能、`nx new` / `nx make`、路由/控制器/校验/模型/Policy、React 页面与 Action。

## 铁律：禁止前后端分离写法

| 禁止 | 正确 |
|------|------|
| 新建独立 `frontend/` / Next / Vite 根 SPA 调 REST | 页面放 `app/src/views/pages/`，由控制器渲染 |
| 手写 `fetch('/api/posts')` + 自建 JSON API 当主路径 | `#[server]` + `generated/actions/*`，或经典表单 POST |
| 在 TS 里再定义一份与 Rust 重复的 DTO | `#[derive(ViewData)]` / `FormField` → 用 `views/generated/*` |
| 手改 `views/generated/**`、`routes.ts`（运行时生成） | 改 Rust 源，再 `cargo build` / 启动刷新 |
| props 塞 `userId` / `isVip` / roles / token | `AuthView` 服务端分支，只下发展示数据 |
| 信表单里的 `user_id` 做授权 | `Post::find` + `authorize(&user, &PostPolicy, …)` |
| 经典 POST 不带 CSRF | `<CsrfField />`；Action 客户端会自动带 |
| 登出用 GET | `POST /logout` + CSRF（或 `logout` Action） |

可选 Bearer JWT 给 **API/移动端**；浏览器主路径仍是 Cookie + 页面/Action，不是「纯前端 + 无 Cookie REST」。

## 一条功能怎么写

```text
Task Progress:
- [ ] 路由：app/src/routes/web.rs（页面 GET / 经典 POST；Action 不必注册）
- [ ] 控制器：薄；req.view / redirect_* / Result<_, AppError>
- [ ] 写库：services/；错误用 AppError（勿 String）
- [ ] 校验：validators/ FormRequest（Action 内 from_values）
- [ ] 授权：policies/ + authorize（写路径查库后比对）
- [ ] 页面：views/pages/*.tsx；import { … } from '../namix' + route + generated
- [ ] 交互：island + useForm(action) 或 SSR form + CsrfField
- [ ] cargo check -p app；需要时 npm run typecheck
```

### 渲染模式

| 模式 | 用途 |
|------|------|
| `.ssr()` | 展示为主；Rust 正文优先 |
| `.island()` | 要 `useForm` / `Link` 软导航 |
| `.spa()` | 仅客户端挂载 |
| `.ssr_html(html)` | 纯服务端 HTML |

运行时 **无 Node SSR**（见 `docs/SSR-RUST.md`）。

### 写操作两出口

1. **Island**：`#[server(name = "…")]` → TS `import { … } from '../generated/actions/…'`
2. **SSR 表单**：`Route::post` + `form: XxxRequest` + flash；表单内 `<CsrfField />`

### 命名路由

- Rust：`route::main::posts` / `req.see_other_to(route::main::posts)`
- TS：`route.posts()` / `route.posts.submit()` — **勿硬编码路径**

### 目录（业务）

```text
app/src/
  controllers/  routes/web.rs  middleware/
  policies/     # nx make policy；目录存在即编译
  validators/   models/  services/
  events/  listeners/  seeders/
  views/pages|components|lib|namix.ts|generated/
```

lean 默认只有 controllers/routes/middleware/views；其余见 `docs/FEATURES.md`。

## 决策树

- **新页面** → 控制器 `req.view` + `views/pages/x.tsx` + `web.rs` 注册 GET  
- **可交互表单** → `.island()` + `#[server]` + `useForm`  
- **无 JS 表单** → `.ssr()` + POST + `CsrfField` + FormRequest 提取器  
- **改/删资源** → 路径或表单只带 id/内容 → 查库 → `authorize` → Service  
- **邮件/短信** → `Mail`/`Sms` 门面；示例 `/mailbox`（`docs/09-mail-sms.md`）  
- **副作用** → `dispatch` / `listen`（`docs/10-events.md`）  
- **CRUD 七件套** → `resource("posts", Ctrl)` 或手写 POST（SSR 友好）

## CLI

```bash
nx new my-app
nx make controller Posts
nx make policy Post
nx make resource Posts
nx make validator PostForm
nx make job WelcomeMail
nx dev -p 3000
```

## Anti-patterns（再强调）

- 把 Namix 写成「Rust API 仓库 + 另一个 React 仓库」
- 为每个页面再包一层 axios/react-query 调自建 REST（除非用户明确只要 JSON API）
- 复制 Laravel Eloquent 魔法到 Toasty（用项目里已有的 `User::find` / `db::run` / `toasty::create!`）
- 跳过 `docs/07-authorization.md` / `docs/05-frontend.md` 的 CSRF 与零授权 props 约定

## Progressive disclosure

- 总索引 → 仓库 [`docs/README.md`](../../../docs/README.md)
- 控制器 / 路由 / 前端 / 授权 → `docs/01`–`07`
- 平台 / 邮件 / 事件 / JWT·Crypt → `docs/08`–`11`
- 架构决策 → `docs/DECISIONS.md`
