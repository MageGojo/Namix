# Namix

**Namix 是一个 Rust 全栈框架工作区**：以类型化 HTTP、表单验证、命名路由、React SSR/Island、Toasty 数据模型和 `nx` 开发 CLI 组成单应用开发体验。

业务代码以轻量的自由函数和结构体组织，目标是提供接近 Laravel 的清晰约定，同时保留 Rust 的编译期类型检查。

## 包含什么

- `crates/namix`：业务侧唯一需要依赖的框架门面与启动器。
- `crates/namix-http`：路由、请求/响应、验证、SSE、WebSocket、静态文件与服务端 Action。
- `crates/namix-macros`：`routes!`、`#[route]`、`#[server]`、`FormField` 和页面数据宏。
- `crates/namix-build`：路由名、前端表单字段、页面注册表的构建期生成器。
- `crates/nx`：创建项目、生成代码、迁移、检查、构建和开发服务器 CLI。
- `app`：完整的单应用示例，涵盖认证、资料、文章、SSR/Island、SSE 与 WebSocket 聊天室。

## 快速开始

```bash
# Rust 依赖、格式和测试
cargo fmt --all -- --check
cargo test --workspace --all-features

# 启动示例应用（默认读取 app/namix.toml）
cargo run -p app --bin app

# 前端类型检查与生产构建
cd app
npx tsc --noEmit
npm run build
```

默认应用监听配置见 [`app/namix.toml`](./app/namix.toml)。首次启动时会按开发配置连接 SQLite 并同步 schema；示例种子可通过 `cargo run -p app --bin seed` 执行。

## 最短业务路径

```rust
use namix::prelude::*;

pub fn routes() -> Router {
    routes! {
        "/" => {
            GET "/posts" => posts::index, name: "posts.index",
            POST "/posts" => posts::create, name: "posts.create",
            PATCH "/posts/:id" => posts::update, name: "posts.update",
            middleware: [require_login],
        },
    }
}
```

命名路由会生成 Rust 侧的 `route::main::*` 以及 TypeScript 侧的 `route.*()`；控制器可通过 `req.redirect_to(route::main::posts_index)` 跳转，页面不必硬编码 URI。

更多业务示例见 [`docs/README.md`](./docs/README.md)（含 [授权 Policy/Gate](./docs/07-authorization.md)），架构与质量状态见 [`docs/PROJECT.md`](./docs/PROJECT.md) 和 [`docs/PROGRESS.md`](./docs/PROGRESS.md)。生产发布流程见 [`docs/PRODUCTION.md`](./docs/PRODUCTION.md)。

## AI / Agent 开发

Namix **不是**前后端分离。写功能前先读 [`AGENTS.md`](./AGENTS.md) 与 [`.cursor/skills/namix/SKILL.md`](./.cursor/skills/namix/SKILL.md)：页面由 Rust `req.view` 渲染，交互走 `#[server]` / 经典表单 + CSRF，勿另起 SPA + REST。

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

Namix 已具备 P0 安全底座及 P1/P2 首版开发 API。发布链采用不可变 `dist/<version>`、共享 `dist/data`、候选 PID 就绪校验、`current` 原子切换和旧进程排水。会话由框架 `SessionStore` 提供：默认 `memory` 仅适合单进程开发；生产滚动更新使用 `[session] driver = "file"`（共享数据面）或接入 `redis`，并由 `nx update` 预检强制约束。浏览器用 opaque Cookie，API 可用 HS256 JWT Bearer（`lifetime_secs` / `jwt_lifetime_secs`）；两者共享 `sid`，可一并撤销。
