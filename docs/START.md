# 五分钟上手

Namix 是 **Rust + React 一体全栈**：浏览器打开的页面由 Rust 控制器渲染，不是「Rust API + 独立 SPA」。

更细的对照表见 [README](./README.md)。本页只覆盖：**怎么注册路由、怎么在 Rust / TSX 里引用、页面怎么起。**

---

## 1. 新项目

```bash
nx new my-app
cd my-app
nx dev -p 3000
```

默认 lean：控制器、路由、中间件、页面。数据库 / 校验 / 会话按需打开，见 [FEATURES.md](./FEATURES.md)。

---

## 2. 新页面（推荐这一条命令）

```bash
nx make page Notes
```

一次生成：

- `controllers/notes.rs` — `req.view(Page::Notes)`
- `views/pages/notes.tsx`
- `view.rs` 里的 `Page::Notes` / `view::notes`（cargo check 时 namix-build 会按 tsx 对齐）

然后在 `app/src/routes/web.rs` 挂上：

```rust
GET "/notes" => notes::index, name: "notes",
```

`name: "notes"` 是契约：Rust 跳转、TSX 链接都靠它，不要在页面里写死 `"/notes"`。

---

## 3. 路由怎么写、怎么用

### 注册（Rust，`web.rs`）

```rust
routes! {
    "/" => {
        GET "/" => home::index, name: "home",
        GET "/login" => auth::login, name: "login",
        GET "/profile/:id" => profile::show, name: "profile",
    },
}
```

短页面也可以直接写闭包：`GET "/greeting" => || "Hello World", name: "greeting",`。

`Route` 是 **注册用的构建器**（`Route::get`）。名字不要叫 `Route::Login`，会和它撞名。

### Rust 里引用（跳转 / 拼 URL）

业务文件顶部：

```rust
use crate::prelude::*;
```

会带上 `AppRoute`（命名路由枚举）和 `Page`（页面名枚举）：

```rust
req.view(Page::Login);
req.redirect_guest_to(AppRoute::Login);
req.redirect_ok_to(AppRoute::Me);

AppRoute::Home.href()                          // → "/"
AppRoute::Profile.to(&[("id", "1")])           // → Some("/profile/1")；下一刀会收成类型化参数
```

旧写法仍然可用，和枚举是**同一个值**：

```rust
req.redirect_ok_to(route::main::me);   // ≡ AppRoute::Me
req.view(view::login);                 // ≡ Page::Login
```

校验失败回表单页：

```rust
FormRedirect::named(AppRoute::Login)   // 也可 FormRedirect::Named("login")
```

### TSX 里引用（链接 / 表单 action）

一律从 `namix` 进，不要直接 `import { route } from '../routes'`：

```ts
import { Link, route, AppRoute } from '../namix'

<Link href={route.login()}>登录</Link>
<Link href={route(AppRoute.Login)}>登录</Link>          // 同上
<Link href={route.profile({ id: 1 })}>资料</Link>
```

`route.login()` 适合日常写；`AppRoute.Login` 适合要补全、或把名字当值传来传去。

**现状**：Rust `AppRoute` 在 `cargo build` 时生成；TS `views/routes.ts` 要等后端启动（或 `nx export routes`）才重写（`@generated`，勿手改）。新页面若编辑器报红，先启动一次或 `nx export routes`，不要手写 `"/login"`。下一刀会在编译期写出 `routes.ts`，见 [NEXT.md](./NEXT.md) DX 节。

---

## 4. 一张对照表

| 你在 `web.rs` 写的 | Rust | TSX |
|--------------------|------|-----|
| `name: "home"` | `AppRoute::Home` / `route::main::home` | `route.home()` / `AppRoute.Home` |
| `name: "login"` | `AppRoute::Login` | `route.login()` / `AppRoute.Login` |
| `name: "me.submit"` | `AppRoute::MeSubmit` / `route::main::me_submit` | `route.me.submit()` / `AppRoute.MeSubmit` |
| `views/pages/posts.tsx` | `Page::Posts` / `view::posts` | 文件名即页面名，不必再写字符串 |

点号会变成 PascalCase：`posts.update` → `AppRoute::PostsUpdate`。

---

## 5. 写操作（先记住两出口）

| 场景 | 做法 |
|------|------|
| 按钮、Island 表单 | `#[server]` + TS `useForm(action)` |
| 无 JS 的 SSR 表单 | `POST` + `<CsrfField />` + `FormRequest` |

不要另起 `/api/posts` REST 当主路径。授权、CSRF、零授权 props 见 [01](./01-controllers.md) / [07](./07-authorization.md)。

---

## 下一步

- 路由细节、中间件、resource：[02-routes.md](./02-routes.md)
- 控制器与 `req.view`：[01-controllers.md](./01-controllers.md)
- 前端 `useForm` / `Link`：[05-frontend.md](./05-frontend.md)
- 校验 unique / 文件：[03-validators.md](./03-validators.md)
- 邮件验证 + `nx work`：[09-mail-sms.md](./09-mail-sms.md) / [08-platform.md](./08-platform.md)
- 清构建缓存：`nx clean`（`target` / `node_modules` / `public/build`；`-n` 只预览）
- 下一刀（编译期路由、表单合一）：[NEXT.md](./NEXT.md) DX 节
- 可选 HTML 404：[ERRORS.md](./ERRORS.md)
