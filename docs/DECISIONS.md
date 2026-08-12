# 技术决策

## 2026-08-05：路由采用宏 DSL，保留链式 API

`Route`/`Router` 链式 API 保留给动态构造和底层场景；业务路由默认使用 `routes!`。它将大量 `.merge(Route::… .register())` 压缩为可分组的声明式表，同时继续产出同一份命名路由 catalog。DSL 现覆盖 HTTP、PATCH 和 WebSocket；WebSocket 不接受 HTTP 中间件，以避免产生看似生效但实际不进入 Upgrade 流程的配置。

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

## 2026-08-10：子路径挂载用 ASSET_PREFIX，不写死 `/build`

血的教训（live-relay 外网 `/lr` 白屏）：HTML 写死 `/build/…`，反向代理只转发 `/lr*` 时，浏览器拉 JS 落到错误上游 → 404 → island 不水合。

框架约定：

- 运行时 `NAMIX_ASSET_PREFIX=/lr`（或 `NAMIX_ASSET_BASE=/lr/build`）生成标签 `/lr/build/…`，并额外注册同路径静态路由；根 `/build/*` 始终保留。
- Vite 脚手架 `base` 读取同一组环境变量，与标签一致。
- 磁盘目录仍是 `public/build/`，与 URL 前缀解耦。
