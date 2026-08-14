# Namix 项目结构

## 架构

```text
app（业务示例）
  └─ namix（启动、配置、DB 与事件门面）
       └─ namix-http（HTTP、路由、验证、实时能力）
            └─ namix-macros（业务语法糖）
```

`app` 可采用单应用扁平布局或多应用（www/user/admin）。`nx new` 默认 lean：只脚手架 **controllers / routes / middleware / views**；Model、Service、Validator、Seeder、Event 由 `namix.toml [features]` 与 Cargo features 打开，详见 [`FEATURES.md`](./FEATURES.md)。仓库示例 `app/` 打开了完整业务面供对照。构建脚本扫描已开启的业务目录，生成 Rust 路由名、TypeScript 表单字段和页面注册表。

## 重要目录

| 路径 | 责任 |
|---|---|
| `crates/namix` | 公共门面、`Boot`、配置、数据库与事件运行时 |
| `crates/namix-http` | 请求/响应、路由、表单验证、SSE、WebSocket、Action |
| `crates/namix-macros` | 路由、Action、视图数据和字段过程宏 |
| `crates/namix-build` | 从业务目录生成 TypeScript/Rust 衍生文件 |
| `crates/nx` | 项目生成、开发、迁移、导出、发布与诊断 CLI |
| `app/src` | 示例业务：路由、控制器、验证器、服务、模型、页面 |
| `app/src/views` | React 页面与生成的前端绑定 |
| `app/database` | Toasty 迁移定义与快照 |
| `docs` | 业务开发文档（01–11 + FEATURES/ERRORS/SSR）、架构、进度、决策和路线图 |

## 安全底座

- `Boot` 默认安装 CSRF/Origin 保护；服务端页面可通过 `namix::csrf::hidden_field(&req)` 输出表单 token，生成的 Action 客户端自动携带同一 token。
- 限流使用 socket peer IP；会话水合后调用 `set_user_subject`，Action 与上传路径便可以按用户分桶。反向代理部署需要在边界验证后再显式覆写 client IP。
- `[security]` 在生产环境强制 HTTPS（或显式 `tls_terminated_by_proxy = true`）、CSRF、Action 密封、禁用启动时 schema push，并要求 `security.session_secret` 或 `NAMIX_SESSION_SECRET`。
- `[session]` 选择持久化驱动：默认 `memory`（单进程开发）；生产滚动更新使用 `file`（共享 `dist/data`）或应用接入的 `redis`。`lifetime_secs` / `jwt_lifetime_secs` 分别控制 Cookie 与 Bearer JWT 时长。Cookie 签名在应用层；JWT 由框架 `namix::Jwt`（HS256）签发，claims 含 `sid` 以便撤销。`nx update` 对滚动路径强制预检共享会话。
- `namix::Crypt` 密封 Flash；`require_guest` + `AuthView` 保证页面 props 不含授权字段。

## 运行假设

- 工作区使用 Rust 2024 edition 和 Tokio 多线程运行时。
- 示例应用默认 SQLite 且打开完整 `[features]`；**新项目**默认 `database.enabled = false`、仅 `pages = true`。连接与页面开关均在 `namix.toml`，驱动体积在 `Cargo.toml` 的 namix features。
- `app/storage/`、构建产物、数据库快照和前端依赖为本地生成文件，默认不纳入版本控制。
- 新代码的最低质量门槛是 `rustfmt`、全特性 Clippy（`-D warnings`）、Rust 全工作区测试与前端 TypeScript 检查。

## 生成链

1. `app/build.rs` 调用 `namix-build::sync_single()`。
2. 构建脚本同步模块声明并扫描路由、验证器、页面与 `#[server]` Action。
3. 生成 `AppRoute` / `route::main::*`、`Page` / `view::*`、`views/generated/fields.ts`、`views/generated/registry.ts` 与 Action TS 客户端。
4. `Boot` 在启动时导出命名路由 JSON，前端据此生成 `views/routes.ts`（与第 3 步时间线不一致：Rust 编译期有 `AppRoute`，TS 要等启动或 `nx export routes`）。计划改为 cargo 同步写出 `routes.ts`，见 [`NEXT.md`](./NEXT.md)。

生成文件带有 `@generated` 标记；修改源路由、验证器或页面后应重新执行 Rust 构建或检查。

## P1/P2 平台能力

- `resource`、`Paginator`/`QueryOptions`、`Policy`/`Gate` 用于统一 CRUD、查询与授权语义。写路径：`authorize(actor, policy, ability, Some(&resource_from_db))`——会话身份对照库记录归属，不信前端自称的 `user_id`。详见 [`07-authorization.md`](./07-authorization.md)。
- `Cache`、`Queue`、`Storage` 均通过 driver trait 抽象；默认内存/本地实现可在开发直接运行。`StorageError` 提供可匹配的策略/I/O 类别，队列 Job 使用可保留上下文链的 `anyhow::Result`。
- `TestClient` 可对路由、Cookie、表单、Action 和 WS 路由做进程内测试；`AppError::internal` 保留 source chain 并安全地映射为 500；请求 ID 与 tracing span 为 OpenTelemetry subscriber 提供结构化信号。
- `nx make` 可生成 controller、resource、policy、job、mail、notification 和 test 骨架。
- 发布层使用不可变 `dist/<version>`、共享 `dist/data`、候选 PID ready 标记、原子 `current` 指针和优雅排水；稳定生产配置由 `dist/data/namix.toml` 经 `NAMIX_CONFIG` 注入；滚动更新前检查共享 Session Store。详见 [`PRODUCTION.md`](./PRODUCTION.md)。
