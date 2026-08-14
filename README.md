# Namix

**Namix 是一个 Rust + React 一体全栈框架**：类型化 HTTP、表单验证、命名路由、SSR/Island、Toasty 模型和 `nx` CLI。页面由 Rust 控制器 `req.view` 渲染，不是「Rust API + 独立 SPA」。

业务用轻量自由函数和结构体，约定接近 Laravel，同时保留编译期类型检查。

## 包含什么

- `crates/namix`：业务侧唯一需要依赖的框架门面与启动器。
- `crates/namix-http`：路由、请求/响应、验证、SSE、WebSocket、静态文件与服务端 Action。
- `crates/namix-macros`：`routes!`、`#[route]`、`#[server]`、`FormField` 和页面数据宏。
- `crates/namix-build`：路由名、前端表单字段、页面注册表的构建期生成器。
- `crates/nx`：创建项目、生成代码、迁移、检查、构建、清理和开发服务器 CLI。
- `app`：完整单应用示例（认证、资料、文章、邮箱验证、后台用户表、SSR/Island、SSE 与 WebSocket 聊天室）。

## 快速开始

```bash
# 质量门槛
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features

# 示例应用（默认读 app/namix.toml）
cargo run -p app --bin app

# 或
cargo run -p nx -- dev -p 3000

# 前端
cd app && npx tsc --noEmit && npm run build
```

默认监听见 [`app/namix.toml`](./app/namix.toml)。首次启动会按开发配置连 SQLite 并同步 schema；种子：`cargo run -p app --bin seed`。`nx clean` 删除 `target/`、`app/node_modules/`、`app/public/build` 等可再生目录。

## 最短业务路径

```rust
use namix::prelude::*;

pub fn routes() -> Router {
    routes! {
        "/" => {
            GET "/" => home::index, name: "home",
            GET "/greeting" => || "Hello World", name: "greeting",
            GET "/posts" => posts::index, name: "posts",
            POST "/posts" => posts::create, name: "posts.submit",
            middleware: [require_login],
        },
    }
}
```

- 控制器是模块函数：`posts::index`（对齐 Laravel `[PostsController::class, 'index']`）。
- 短响应可写同步闭包，不必再包 `async fn`。
- `name:` 生成 Rust `AppRoute::Posts` / `route::main::posts` 和 TS `route.posts()`；跳转用 `req.see_other_to(AppRoute::Posts)`，页面不要写死路径。
- 当前用户：参数 `user: AuthUser`，或 `req.user()`（`use crate::prelude::*`）。字段：`req.input("title")`。
- 写操作：Island 用 `#[server]`（自动挂 `POST /api/a`）；SSR 表单用 `POST` + `<CsrfField />`。
- 用户上传：`Storage::disk("local")?` + `put_file` / `put_with_policy`；公开文件 `public` disk + `nx storage link`。详见 [`docs/08-platform.md`](./docs/08-platform.md) §5。

## 出站 HTTP（服务器里调第三方）

框架没有 `Http::get`。在 **Namix 进程**里用业务包 `reqwest`，写在 `services/`。`#[server]` 可以调这个 Service；密钥不会自动到浏览器，但 `ActionOk` 里 return 的字段会。只映射展示 DTO，不要把对方整包 JSON 转给前台。详见 [`docs/08-platform.md`](./docs/08-platform.md) §7 与 [`docs/01-controllers.md`](./docs/01-controllers.md)。

实时推送用 SSE / `Route::ws`，不是 Laravel Echo。进程内副作用用 `dispatch` / `listen`。

## 文档

业务怎么写：[`docs/README.md`](./docs/README.md)（先看 [`docs/START.md`](./docs/START.md)）。  
授权 Policy：[`docs/07-authorization.md`](./docs/07-authorization.md)。  
架构与进度：[`docs/PROJECT.md`](./docs/PROJECT.md)、[`docs/PROGRESS.md`](./docs/PROGRESS.md)。  
生产发布：[`docs/PRODUCTION.md`](./docs/PRODUCTION.md)。

## AI / Agent 开发

写功能前先读 [`AGENTS.md`](./AGENTS.md) 与 [`.cursor/skills/namix/SKILL.md`](./.cursor/skills/namix/SKILL.md)：页面由 Rust `req.view` 渲染，交互走 `#[server]` / 经典表单 + CSRF，勿另起 SPA + REST。

## 常用质量命令

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
(cd app && npx tsc --noEmit && npm run build)
cargo build --workspace --all-features
cargo build -p app --profile release-min --bin app
```

## 当前边界

P0 安全底座及 P1/P2 首版开发 API 已具备。发布链：不可变 `dist/<version>`、共享 `dist/data`、候选 PID 就绪校验、`current` 原子切换、旧进程排水。会话默认 `memory`（单进程开发）；生产滚动用 `[session] driver = "file"` 或接入 `redis`，`nx update` 会预检。浏览器用 opaque Cookie，API 可用 HS256 JWT Bearer；两者共享 `sid`。

尚未做成框架门面的：真 SMTP、真 OAuth、出站 `Http::`（业务包直接 `reqwest`）、内置 S3/FTP SDK（用 `Storage::extend`）、Redis 限流客户端。路线图见 [`docs/NEXT.md`](./docs/NEXT.md)。
