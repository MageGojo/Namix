# 技术决策

## 2026-08-05：路由采用宏 DSL，保留链式 API

`Route`/`Router` 链式 API 保留给动态构造和底层场景；业务路由默认使用 `routes!`。它将大量 `.merge(Route::… .register())` 压缩为可分组的声明式表，同时继续产出同一份命名路由 catalog。DSL 现覆盖 HTTP、PATCH 和 WebSocket；WebSocket 不接受 HTTP 中间件，以避免产生看似生效但实际不进入 Upgrade 流程的配置。短响应可用同步闭包：`GET "/greeting" => || "Hello World"` / `Route::get("/greeting", || "Hello World")`，不必为纯文本再包一层 `async fn`。

## 2026-08-05：跳转目标只使用站内绝对路径

`redirect` 参数、`previous_url` 和表单校验共享 `is_local_path`，拒绝完整 URL、协议相对 URL、反斜杠变体与控制字符。业务表单用 `Rule::LocalPath` 与 `Validated::local_path_or` 表达此要求。

## 2026-08-05：密码写入使用 Argon2id，保留旧演示数据兼容

新注册密码使用带随机盐的 Argon2id PHC 字符串。旧示例数据库内的 SHA-256 固定盐格式仅用于验证兼容，不再用于写入；下一阶段在成功登录后自动重哈希。

## 2026-08-05：全特性 Clippy 作为合并门槛

工作区采用 `cargo clippy --workspace --all-targets --all-features -- -D warnings`。对于 HTTP 层直接以 `Response` 作为错误值的少数 API，保留局部 `result_large_err` 说明，因为避免二次错误转换比装箱更符合公开框架 API 的使用体验。

## 2026-08-05：监听器错误不能被当作正常关机

`Server::run` 等待 Ctrl-C、终止信号或任一 listener 完成。此前第一个 listener 的 I/O 错误会在等待阶段被丢弃，随后显示为普通退出。现在该错误会向应用上浮，同时通知并等待其余 listener 停止，从而让端口冲突、TLS/QUIC 绑定失败等问题可诊断。

## 2026-08-06：会话持久化与发布重叠解耦

Cookie 签名留在应用边界；会话记录通过框架 `SessionStore` 持久化。默认 `memory` 保持单进程开发简单性。单机生产优先 `file` 驱动写入共享 `dist/data/storage`，与不可变版本目录对齐且无需额外中间件；多机或已有 Redis 时使用 `RedisSessionStore`。`nx update` 在滚动路径上拒绝进程内会话，避免新旧 PID 交叠时认证状态分裂。

## 2026-08-06：浏览器 Cookie 与 API JWT 双通道

浏览器继续使用 opaque 签名 Cookie（`namix_session`，`Max-Age = [session].lifetime_secs`）。API / 移动端使用框架 `namix::Jwt`（HS256），claims 含 `sid`，密钥复用 `session_secret`，时长为 `jwt_lifetime_secs`。登录/注册 Action 同时写 Cookie 并返回 `access_token`；`hydrate` 接受 Cookie 或 `Authorization: Bearer`（opaque 与 JWT 均可）。登出与全设备撤销按 `sid` 清理 Store，两条通道一并失效。

## 2026-08-06：Crypt、访客门禁与零授权 props

- `namix::Crypt`（AES-256-GCM + HKDF）自动密封 Flash；密钥由 `session_secret` 派生，解密只在服务端。
- `require_guest` 阻止已登录用户进入 `/login`、`/register`。
- `AuthView` 在控制器内按身份分支；页面只接收展示数据（如 `navLinks` / `greeting`），禁止下发 `isVip` / `userId` / roles。

## 2026-08-06：Policy 授权对齐 Laravel authorize

写操作以「会话中的 Actor」对照「数据库加载的 Resource」：`authorize(&user, &PostPolicy, Ability::Update, Some(&post))`。前端只提交资源 id 与内容字段；body 中的 `user_id` / 角色声明不可信。文档见 [`07-authorization.md`](./07-authorization.md)。

## 2026-08-13：框架审计后的安全与 DX 收口

- Action 包络强制 `ts`（缺省 400、过期 410），避免明文重放。
- 读 body 失败返回 400，不再静默当空表单。
- HTTP/3 关闭 0-RTT；短信 OTP 改 CSPRNG；限流 `max_requests=0` 视为关闭通道。
- CSRF `hidden_field` 跟随 `CsrfConfig.form_field`；提取器支持 4–5 元参数；`resource` 的 update 同时绑 PUT/PATCH。
- `OneTimeTokenStore`（memory/file）供密码重置等一次性令牌跨重启使用。
- CI 以 Clippy `-D warnings` 为门禁：`WsHandshakeOutcome` 装箱 `Request`，避免大枚举。

## 2026-08-13：页面名与路由名同一套生成常量

`req.view("login")` 仍可用，但业务默认 `req.view(Page::Login)`（`view::login` 为别名）。`app/src/view.rs` 由 namix-build 根据 `views/pages/*.tsx` 生成。新页面用 `nx make page`，不要先 `make controller` 再手配 TSX。

命名路由生成 **`AppRoute` 枚举**（不要叫 `Route`，以免挡住 `Route::get`）。`route::main::login` 是同一枚举值的常量别名。TS：`route.login()` 与 `route(AppRoute.Login)` 等价。带参：`AppRoute::Profile.to(&[("id", "1")])`。入门：[START.md](./START.md)。

## 2026-08-13：HTML 错误页显式注册、默认不强制

自定义 404/403/500 是可选的。不扫描 `errors/404.tsx`；要在 `routes()`（或 `Boot`）上 `.error_page` / `.error_pages`。JSON 与 Action 永远不走 HTML 页。具体状态优先于 catch-all。

## 2026-08-13：文档壳由 Rust 产出

`<html>` / `<body>` 由 Rust 文档壳产出。默认能力是**任意属性**（`data-theme`、`style`、`id`）和 **`<head>` 片段**；`class` 只是可选糖。暗亮色走 `data-theme` + `color-scheme` + 文档级 CSS，页面不必写 `dark:`。需要整份 HTML 时用 `.template(...)` 或 `.template_file("src/views/layouts/app.html")`（占位符 `{{html_attrs}}` / `{{app}}` 等）。TSX `<Head>` 只服务软导航后的标题/meta。

血的教训（live-relay 外网 `/lr` 白屏）：HTML 写死 `/build/…`，反向代理只转发 `/lr*` 时，浏览器拉 JS 落到错误上游 → 404 → island 不水合。

框架约定：

- 运行时 `NAMIX_ASSET_PREFIX=/lr`（或 `NAMIX_ASSET_BASE=/lr/build`）生成标签 `/lr/build/…`，并额外注册同路径静态路由；根 `/build/*` 始终保留。
- Vite 脚手架 `base` 读取同一组环境变量，与标签一致。
- 磁盘目录仍是 `public/build/`，与 URL 前缀解耦。

## 2026-08-14：下一阶段做 Rust ↔ React 写体验，不堆功能面

Laravel 式能力（校验 unique、队列、角色、邮件验证、i18n、后台表）第一版已落地。再接真 SMTP / 真 OAuth / 更重后台套件，对「每天写一个页面」帮助很小。

下一刀优先缩短 **Rust 写完 → TSX 能点、类型能对上** 这一圈：编译期 `views/routes.ts`、带参路由类型、`Link` 吃命名路由、表单 JSON/文件同一套、`ActionOk<T>` 进 TS。校验稳定码已落地（`username.taken` + `lang/*.json`）。文件存储命名磁盘已补齐，见 [`08-platform.md`](./08-platform.md) §5。不引入 Redis。完整排序见 [NEXT.md](./NEXT.md) DX 节。

## 2026-08-14：Storage 对标 Filesystem，不内置对象存储 SDK

`Storage::disk` / `fake` / `nx storage link` 走 `[storage]` 配置。本地盘防路径穿越与符号链接、HMAC 临时 URL。S3 只保留 `S3Transport` 口子；FTP/SFTP 用 `Storage::extend`。不把 AWS SDK 或 ftp crate 编进框架默认依赖。
