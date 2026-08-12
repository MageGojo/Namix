# 开发进度

## 2026-08-05：框架语法与质量审计

完成项：

- 统一全工作区格式，并修复全特性 Clippy 报出的可读性、错误类型和 API 语义问题。
- `Range` 解析从 `Result<Option<_>, ()>` 收紧为 `Result<ByteRange, RangeParseError>`，移除冗余状态并提供可诊断的错误。
- 增加 `ByteRange` / `ContentRange` 的空范围语义，清理不必要借用和重复嵌套控制流。
- 扩展 `routes!`：支持 `PATCH` 与 `WS`，并将示例应用路由表从重复 `.merge(Route::…register())` 收敛为分组 DSL。
- 增加 `Route::patch` 与 `Router::patch` 便捷 API。
- 新增 `Rule::LocalPath`、`Validated::local_path_or` 与 `is_local_path`，使登录后跳转和返回页只接受站内路径。
- 将示例应用的新密码哈希迁移为随机盐 Argon2id，保留旧 SHA-256 示例库的登录兼容校验。
- 修复聊天室同一账号多连接时的在线状态计数，避免一个标签页断开就将仍在线用户移出名单。
- 服务监听器启动失败现在会保留原始 I/O 错误并清理其余监听器，不再被误报为普通的 `listener exit`。
- 禁用 `bubbletea-widgets` 未使用的剪贴板默认特性，移除 `block` 依赖产生的 Rust future-incompatibility 警告。
- 新增覆盖范围解析、站内跳转、路由 DSL、Argon2id 与旧哈希兼容的测试。

验证完成：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`（14 个实际单元/集成测试通过）
- `cd app && npm run typecheck`
- `cd app && npm run build:release`（前端生产构建、WASM、`release-min` 后端编译）
- `nx doctor --check`（所有项目结构检查通过）
- 隔离端口下的 release 二进制冒烟：路由 catalog、登录页渲染及受保护聊天室重定向均通过。

## 2026-08-06：P0 安全与会话底座

完成项：

- 新增默认启用的 CSRF 中间件：安全方法签发 SameSite 严格、可读 token Cookie；浏览器写请求同时校验同源 `Origin` 与 header/form token。`#[server]` 的 WASM 客户端自动回传 `X-CSRF-Token`。
- 新增内存限流器与可复用策略：按 socket 客户端 IP 或已水合用户限流，Action 自动区分 login、registration、general action，multipart 与 upload 路径单独限流并返回 `429 + Retry-After`。
- `AppError` 作为统一领域错误边界，控制器、JSON、Action 共享状态码及 `{ error, message, errors }` 协议；内部错误不再把底层详情返回给调用方。
- 配置加入 `[security]` 与 `[security.rate_limit]`，启动前校验端口、数据库驱动、生产 HTTPS、CSRF、Action seal、迁移策略和会话密钥。
- 会话升级为 CSPRNG 不透明 ID 加签 Cookie，包含绝对过期、登录轮换、全设备撤销、密码重置一次性 token，以及旧 SHA-256 成功登录后的 Argon2id 自动重哈希。

验证：

- `cargo test --workspace --all-features`

## 2026-08-06：P1/P2 框架 API 第一版

- 资源路由：`resource("posts", PostsController)` 生成七条 CRUD 路由与 `posts.*` 命名路由。
- 查询层：`Paginator<T>`、`QueryOptions` 和 `SortWhitelist` 为分页、筛选及安全排序提供统一载荷。
- 授权：`Policy`、`Gate`、`authorize` 可以在 SSR、API 和 Action 共用。
- 基础设施：内存 Cache、异步内存 Queue、异步事件监听、Local/S3-compatible Storage 与临时 URL 合同。
- 工程能力：`TestClient` 覆盖路由、Cookie、表单、Action 与 WebSocket 路由握手；通知驱动、请求 ID、结构化耗时日志和数据库计时已接入。
- CLI：`nx make resource|policy|job|mail|notification|test` 提供对应骨架。

## 2026-08-06：发布链与业务边界自检

- 将示例 `UserService` 的写路径从字符串错误升级为 `AppError`；注册 Action 不再按错误文本分支，控制器仅处理业务结果。
- 发布版本改为 staging 后再落盘，拒绝覆盖既有版本；`current` 与 `LATEST` 通过同目录临时文件原子切换。
- `nx update --build` 先构建候选、不碰 current；新 PID 在所有 TCP listener 成功绑定后写 ready 标记，确认后才切换指针并排水旧 PID。
- 修复共享 pidfile 的排水竞争：旧进程只会清理仍指向自身的 pidfile。
- 发布包改为原样携带开发配置；生产稳定配置放入 `dist/data/namix.toml`，启动器以 `NAMIX_CONFIG` 注入，支持受信任反向代理的 TLS 终止。
- 新增 `/__namix/health`、生产配置/Caddy 示例及 `ops/deploy-release.sh` 本地到服务器的不可变上传流程。

验证：

- `cargo test -p nx -p namix -p namix-http --all-features`
- `cargo check -p nx -p namix -p app`

## 2026-08-06：错误链与类型化基础设施

- 引入 `thiserror` 与 `anyhow`：公共框架边界使用可匹配的错误类型，队列 Job 保留任意底层错误及 context chain。
- `AppError::Internal` 改为保存 source；HTTP 与 Action 仍返回通用 500，tracing 记录完整内部链。
- `ConfigError` 区分 TOML parse source 与聚合验证错误；`StorageError` 区分键、上传策略、I/O、时钟和后端错误，并自动映射为正确的 `AppError`。
- `WsError` 改为 JSON 序列化与传输错误枚举。

详见 [`ERRORS.md`](./ERRORS.md)。

## 2026-08-06：共享 Session Store 与滚动更新预检

- 框架新增 `Session` / `SessionStore`：`MemorySessionStore`（开发默认）、`FileSessionStore`（经 `dist/data/storage` 共享）、`RedisSessionStore`（`RedisBackend` 适配）。
- `[session]` 配置接入 Boot；生产默认拒绝 `memory`，可用 `NAMIX_ALLOW_MEMORY_SESSIONS=1` 显式接受维护窗口冷切。
- 示例 `SessionService` 负责 Cookie HMAC 签名与 JWT 配对签发；持久化与撤销委托框架 Store。
- `nx update` 在存在旧进程时强制检查共享会话驱动，避免交叠窗口会话丢失。

## 2026-08-06：可配置会话时长与 JWT Bearer

- `[session] lifetime_secs` / `jwt_lifetime_secs` 控制 Cookie 与 API access token 过期；`SessionService::cookie_options_for(ttl)` 支持单次自定义 Max-Age。
- 框架 `namix::Jwt`（HS256）与 `session_secret` / Session Store 打通：JWT 携带 `sid`，登出与全设备撤销对 Cookie 与 Bearer 同时生效。
- 登录/注册 Action 在写 Cookie 的同时返回 `access_token` / `token_type` / `expires_in`。

## 2026-08-06：Crypt + 访客门禁 + 零授权前端

- `namix::Crypt` 密封 Flash；`require_guest` 保护登录/注册页。
- `AuthView` 服务端分区渲染；首页 props 改为 `greeting` + `navLinks`，移除 `username` / `isVip`。

## 2026-08-06：授权文档（Policy vs 数据库资源）

- 新增 [`07-authorization.md`](./07-authorization.md)：说明 Laravel 式 `authorize`——会话身份对照 DB 资源归属；更新 README / 控制器 / 安全边界 / 模型文档交叉引用。

## 2026-08-10：子路径静态资源（反哺自 live-relay `/lr` 白屏）

- `namix-http`：`NAMIX_ASSET_PREFIX` / `NAMIX_ASSET_BASE` → HTML 标签与 `/…/build/*` 别名路由；manifest 剥离带前缀的 Vite `base`。
- `nx` Vite 模板：`productionAssetBase()` 与上述环境变量对齐。
- 文档：[`05-frontend.md`](./05-frontend.md)、[`02-routes.md`](./02-routes.md)、[`DECISIONS.md`](./DECISIONS.md)。
- 验收：`cargo test -p namix-http --features pages assets::` 单测；默认行为仍为 `/build`。

## 2026-08-12：教学文档全量对齐 + PostPolicy 示例

- 示例应用落地 `PostPolicy`：`create` / `update` / `destroy` 经 `authorize`；SSR 表单补 `CsrfField`；路由 `posts.update` / `posts.destroy`。
- 教学系列纠错：登出 POST+CSRF、Service `AppError`、TrustedProxies、Action 路径 `/api/a`。
- 新增 [`08-platform.md`](./08-platform.md)、[`09-mail-sms.md`](./09-mail-sms.md)、[`10-events.md`](./10-events.md)、[`11-jwt-crypt.md`](./11-jwt-crypt.md)；扩写 06 聊天室、07 按真实代码、ERRORS / README 索引。
- 验收：`cargo test -p app --lib policies::post_policy`。
