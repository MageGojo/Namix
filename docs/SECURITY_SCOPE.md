# 安全与隐私边界

本仓库的安全验证限定在本地示例应用、框架单元测试与开发数据库。测试数据使用示例账号和本地 SQLite 文件；不提交真实账号、密钥、Cookie、JWT、外部服务地址或生产数据库。

提交代码前应确认：

- 密码使用 Argon2id 生成的哈希，视图数据和日志不输出密码、会话令牌或 `access_token`。
- 登录后的 `redirect` 仅为站内绝对路径。
- 新增 cookie 明确设置适合其用途的 `Secure`、`HttpOnly`、`SameSite` 与有效期（`CookieOptions.max_age`，对齐 `[session].lifetime_secs`）。
- Flash 等敏感 Cookie 经 `namix::Crypt`（AES-256-GCM）自动加密封装；解密仅在服务端。
- 页面 props / 前端 JS **不得**包含授权字段（`userId` / `isVip` / roles / token）。身份分支用 `AuthView` + SSR 下发已定稿的展示数据（导航链接、问候语）。
- `#[server]` 调第三方：API Key 与出站请求留在服务器；**不要**把对方整包 JSON 放进 `ActionOk`。`action_seal` 只加密传输，解密后页面仍能看到成功体。需登录的数据在 Action 内查会话。见 [`01-controllers.md`](./01-controllers.md) 与 [`08-platform.md`](./08-platform.md) §7。
- 写操作授权：用会话中的 `LoginUser` / `AuthUser` 与 **数据库加载的资源** 做 `Policy` / `authorize` 比对（≈ Laravel `$this->authorize`）；禁止信任 body/query 里的 `user_id`、`is_admin`。示例见 `PostPolicy` + `posts::{create,update,destroy}`，文档 [`07-authorization.md`](./07-authorization.md)。
- 经典 HTML POST 必须带 `_csrf`（`<CsrfField />`）；`useForm` / Action 客户端自动携带。Bearer-only 请求豁免 CSRF，但仍受限流与鉴权约束。带 HMAC `expires`+`signature` 的 storage PUT（`temporary_upload_url`）按前缀豁免 CSRF，**未签名的 PUT 仍受保护**。
- 私有 disk 不要靠公开 URL 读文件；`GET /storage/private/…` 无签名当 404。用户上传走 `Storage::disk`，密钥与对象根目录不进页面 props。
- Bearer JWT 与 Cookie opaque 等价参与 resolve / revoke；详见 [`11-jwt-crypt.md`](./11-jwt-crypt.md)。
- 任何网络测试仅命中本机或项目明确配置的开发端点。
- 生产部署在启用 cookie / JWT 会话前配置 CSRF/Origin 保护、速率限制、共享 Session Store 和 `[security].trusted_proxies`。
