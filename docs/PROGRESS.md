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

## 2026-08-13：框架审计优化

- Action `ts` 必填、body 读失败硬失败、H3 关闭 0-RTT、OTP CSPRNG、限流 0 预算拒绝。
- CSRF hidden_field 尊重配置字段名；`Path` 优先 `:id`；handler 支持 4/5 个提取器；`resource` update 同时注册 PUT。
- `namix::OneTimeTokenStore`；示例密码重置默认落盘；Me 页去掉 `userId`；SSR 表单下发 `csrfToken`。
- 新增 GitHub Actions：`fmt` / Clippy / 工作区测试 / `app` typecheck。
- `WsHandshakeOutcome` 将 `Request` 装箱以压低枚举体积；storage 签名 URL 测试按参数名取 query。
- 文档壳 API：属性（`.html` / `.body`）+ `.head` + `.template` / `.template_file`；暗亮色默认 `data-theme`，不强制 class。

## 2026-08-13：写业务 DX

- `nx make page Notes`：一次生成控制器、`ViewData`、`views/pages/notes.tsx`，并立刻写入 `mod.rs` / `view.rs`，不必等 cargo check。
- 页面名 `Page::Posts` / `view::posts`（namix-build 生成），写错页面名会编不过。
- 命名路由 `AppRoute::Login`（枚举）+ `route::main::login`（别名）+ TS `route.login()` / `AppRoute.Login`。不叫 `Route`，避免挡住 `Route::get`。
- `req.csrf_token()`、`Option::or_not_found()`；validator 骨架改为 `FormRequest`；`FormRedirect::named(AppRoute::Login)`。
- `app/src/prelude.rs`：`use crate::prelude::*` 一次拿到框架 API + `AppRoute` + `Page`。
- 入门文档 [`START.md`](./START.md)。
- 仓库根 `rust-analyzer.toml`（clippy、只检查当前包）+ `cargo nx` 别名。

## 2026-08-13：可选 HTML 错误页

- `Router` / `Boot`：`.error_page(404, …)` 与 `.error_pages(…)`。不注册则保持框架默认 HTML / JSON。
- 未匹配路由、`AppError` 浏览器响应、`req.not_found()` / `req.forbidden()` / `req.error_response` 共用同一张表；JSON 与 Action 不走 HTML。
- 示例：`controllers/errors.rs` + `views/pages/errors.tsx`；骨架 `nx make error`。
- 具体状态优先于 catch-all；`web.rs` 优先于 `Boot`。

## 2026-08-13：邮件验证 / unique / 队列 / 角色

- 邮箱验证：注册写真实 email，`Mail` log/file 发验证信（不接真 SMTP）；`GET /email/verify`；资料页可重发。
- `Rule::unique` / `exists` + SQLite `PresenceVerifier`；multipart `UploadedFile` + `Image`/`Mimes`/`MaxBytes`；资料页头像上传。
- Durable queue：`[queue] file|sqlite` + `QueuedJob` + 延迟 + `nx work`（`app --bin work`）。
- `User.role` + `namix::access` + `require_admin`；`GET /admin/users` + `DataTable`。
- 短信 `register_transport`、Dev 社交登录 `/auth/dev`、`trans()` / `lang/*.json`。

## 2026-08-14：校验错误改为稳定码

`Rule` / `AppError::validation` 返回 `username.taken` 这类码。`trans_error` 与前端 `t()` 共用 `lang/*.json`（先字段键，再 `validation.{rule}`，`:attribute` 可走 `attributes.{field}`）。经典 POST 的 flash 与 HTML 错误页会翻译；Action JSON 仍传码，由 `useForm` 翻译。`<html lang>` 跟 `[i18n].locale`。`nx new` 带上 `lang/*.json` 与 `t()`。

## 2026-08-14：nx clean

`nx clean` 删除 `target/`、`app/node_modules/`、`app/public/build` 等可再生构建产物。`nx cleen` / `clen` 等常见拼写也能用；`-n` 只预览。

## 2026-08-14：闭包路由

`Route::get("/greeting", || "Hello World")` 与 `routes!` 里 `GET "/greeting" => || "Hello World"`。同步闭包可直接返回 `&str` / `String`，不必再包一层 `async fn`。

## 2026-08-14：Request.input / req.user()

`req.input("title")` 合并 query + JSON / 表单字段。示例应用 `req.user()` → `LoginUser`（`use crate::prelude::*`）。控制器方法仍是 `users::index`，不引入 PHP 的 `[Class, 'index']`。

## 2026-08-14：出站 HTTP 约定

不提供 `Http::get` 门面。服务器里用业务包 `reqwest` + `services/`；`#[server]` 只把展示 DTO 放进 `ActionOk`。文档：[08-platform §7](./08-platform.md#7-出站-http-调第三方)、[01-controllers](./01-controllers.md)。

## 2026-08-14：Storage 命名磁盘

补齐 Laravel Filesystem 常用面：`[storage]` disks、`Storage::disk` / `default_disk` / `fake`、`exists`/`copy`/`files`/`put_file`、visibility、scoped/read-only、HMAC 临时下载/上传 URL、公开 `GET /storage/*`、`nx storage link`、本地 `cover`/`to_webp`。示例头像改走 `Storage::disk("local")`。S3/FTP/SFTP 仍用 `Storage::extend`，不内置 AWS SDK。文档：[08-platform §5](./08-platform.md#5-storage)、[FEATURES §6.2](./FEATURES.md)。

## 2026-08-14：DX 下一刀写进路线图

功能面暂收口。下一阶段按写页面手感排（编译期 `routes.ts`、带参路由、`Link`、表单合一、校验码、Action TS），不堆 SMTP/OAuth/Redis。见 [`NEXT.md`](./NEXT.md) DX 节、[`DECISIONS.md`](./DECISIONS.md)。


