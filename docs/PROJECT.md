# Namix 项目结构

## 架构

```text
app（业务示例）
  └─ namix（启动、配置、DB 与事件门面）
       └─ namix-http（HTTP、路由、验证、实时能力）
            └─ namix-macros（业务语法糖）
```

`app` 采用单应用、扁平 MVC 布局：控制器处理 HTTP，验证器生成合法输入，服务层承载写操作，模型封装读取和关联。构建脚本扫描业务源文件，生成 Rust 路由名、TypeScript 表单字段和页面注册表。

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
| `docs` | 业务开发文档、架构、进度、决策和路线图 |

## 安全底座

- `Boot` 默认安装 CSRF/Origin 保护；服务端页面可通过 `namix::csrf::hidden_field(&req)` 输出表单 token，生成的 Action 客户端自动携带同一 token。
- 限流使用 socket peer IP；会话水合后调用 `set_user_subject`，Action 与上传路径便可以按用户分桶。反向代理部署需要在边界验证后再显式覆写 client IP。
- `[security]` 在生产环境强制 HTTPS（或显式 `tls_terminated_by_proxy = true`）、CSRF、Action 密封、禁用启动时 schema push，并要求 `security.session_secret` 或 `NAMIX_SESSION_SECRET`。
- 默认会话为进程内实现，具备 token 签名、绝对过期和按用户撤销语义；无感滚动发布与生产集群均应替换为共享存储驱动。

## 运行假设

- 工作区使用 Rust 2024 edition 和 Tokio 多线程运行时。
- 示例应用默认 SQLite；`app/namix.toml` 控制连接、HTTPS、HTTP/3、页面及 Action 密封开关。
- `app/storage/`、构建产物、数据库快照和前端依赖为本地生成文件，默认不纳入版本控制。
- 新代码的最低质量门槛是 `rustfmt`、全特性 Clippy（`-D warnings`）、Rust 全工作区测试与前端 TypeScript 检查。

## 生成链

1. `app/build.rs` 调用 `namix-build::sync_single()`。
2. 构建脚本同步模块声明并扫描路由、验证器、页面与 `#[server]` Action。
3. 生成 `route::main::*`、`views/generated/fields.ts`、`views/generated/registry.ts` 与 Action TS 客户端。
4. `Boot` 在启动时导出命名路由 JSON，前端据此生成 `views/routes.ts`。

生成文件带有 `@generated` 标记；修改源路由、验证器或页面后应重新执行 Rust 构建或检查。

## P1/P2 平台能力

- `resource`、`Paginator`/`QueryOptions`、`Policy`/`Gate` 用于统一 CRUD、查询与授权语义。
- `Cache`、`Queue`、`Storage` 均通过 driver trait 抽象；默认内存/本地实现可在开发直接运行。`StorageError` 提供可匹配的策略/I/O 类别，队列 Job 使用可保留上下文链的 `anyhow::Result`。
- `TestClient` 可对路由、Cookie、表单、Action 和 WS 路由做进程内测试；`AppError::internal` 保留 source chain 并安全地映射为 500；请求 ID 与 tracing span 为 OpenTelemetry subscriber 提供结构化信号。
- `nx make` 可生成 controller、resource、policy、job、mail、notification 和 test 骨架。
- 发布层使用不可变 `dist/<version>`、共享 `dist/data`、候选 PID ready 标记、原子 `current` 指针和优雅排水；稳定生产配置由 `dist/data/namix.toml` 经 `NAMIX_CONFIG` 注入。详见 [`PRODUCTION.md`](./PRODUCTION.md)。
