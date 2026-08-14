# 下一阶段路线图

以下按高质量 Rust 全栈框架的收益与风险排序。每项应伴随 API 文档、单元测试和至少一个示例应用用例。

## P0：安全与运行可靠性（已完成第一版）

- CSRF/Origin：浏览器 mutation 强制同源 Origin + double-submit token；Bearer-only API 自动豁免。
- 限流：提供 IP / 已认证用户策略，Action 预设 login、registration、action 三档，上传单独限流。
- 错误边界：`AppError` 映射 HTML、JSON、Action，并保留 `Retry-After`。
- 配置启动校验：生产 HTTPS、会话密钥、Action seal、迁移策略和数据库配置必须一致。
- 会话：Cookie opaque（CSPRNG id + HMAC）与可选 HS256 JWT Bearer（claims 含 `sid`）；`[session] lifetime_secs` / `jwt_lifetime_secs`；`SessionStore`（memory / file / redis）；登录轮换、全设备登出、一次性密码重置、旧 SHA-256 登录后升级 Argon2id。

后续增强：真实 Redis 限流/会话客户端、上传 body 与磁盘配额、跨进程密码重置 token。  
（`TrustedProxies` + `[security].trusted_proxies` CIDR/`X-Forwarded-For` 解析已提供第一版。）

## P1：Laravel 式开发体验（框架 API 第一版已完成）

1. **资源路由**：提供 `resource("posts", PostsController)` 或等价 Rust DSL，生成 index/create/store/show/edit/update/destroy 路由与命名。
2. **分页与查询参数**：标准 `Paginator<T>`、安全排序/过滤白名单、TS 类型同步。
3. **缓存与后台任务**：本地实现起步，抽象 Redis/队列后端；事件监听器可选择同步或排队。
4. **邮件、通知与文件存储抽象**：开发日志驱动 + 可替换生产驱动。
5. **策略与 Gate**：为模型/资源定义 `can`/`authorize`，减少控制器里散落的角色判断。

验收：资源路由、分页/白名单排序、Policy/Gate、内存 Cache/Queue、Storage 已提供统一 API；`nx make` 支持 resource、policy、job、mail、notification、test。

## P2：工程化与可观测性（框架 API 第一版已完成）

1. **HTTP 测试客户端与临时数据库夹具**：覆盖路由、cookie、表单、Action、SSE 协议和迁移。
2. **配置层**：显式环境覆盖、必填密钥校验、敏感配置脱敏输出和多环境 profile。
3. **OpenTelemetry/结构化日志**：请求 ID、Action 名称、数据库耗时、错误链和采样配置。
4. **兼容性矩阵**：稳定 Rust 最低版本、SQLite/PostgreSQL/MySQL 后端与浏览器 SSR 矩阵。

验收：框架单元测试覆盖路由、授权、分页、缓存、队列、存储、通知与测试客户端；后续补 Redis/S3 网络后端、OTel exporter 与 CI 发布矩阵。

## DX：Rust ↔ React 写页面这一圈（优先于再堆功能）

功能面第一版已够用：`AppRoute` / `Page` / `crate::prelude::*`、[START.md](./START.md)、邮箱验证（log/file，不真发 SMTP）、`Rule::unique`/`exists` + multipart 文件、file/SQLite 队列 + `nx work`、角色/`require_admin`、短信 transport 口子、Dev 社交登录、i18n、`DataTable`。

再加真 SMTP、真 OAuth、更重后台套件，对「每天写一个页面」几乎没帮助。下一阶段按 **写代码手感** 排，**不引入 Redis**。决策见 [DECISIONS.md](./DECISIONS.md)（2026-08-14）。

### 现状摩擦

| 点 | 现在 |
|----|------|
| TS 路由契约 | `views/routes.ts` 要等 Boot 或 `nx export routes`；cargo 只写出 Rust `AppRoute`。新页面时编辑器常红，容易手写 `"/me"` |
| 带参 URL | `AppRoute::Profile.to(&[("id", "1")])`，漏 key 运行时才知道 |
| `Link` | 只收 `href: string`，必须先 `route.login()` |
| 表单 | Island `useForm` 走 JSON Action；文件必须经典 `multipart` + `<CsrfField />`（资料页已是两套心智） |
| 校验错误 | **稳定码** `username.taken`；`trans_error` / `t()` / `useForm.messages` 都对码 |
| Action 成功体 | `ActionOk<T>` 没有进 TS，`useForm` / `register()` 几乎是 `unknown` |

### 下一刀（按收益）

1. **路由契约一次生成、两边都能用**（先做这一刀）  
   `cargo build` 写出 `views/routes.ts`；`AppRoute::profile("1")` 代替 `.to(&[("id", "1")])`；`<Link>` 能直接吃命名路由（或统一 `href={route.profile({ id: 1 })}` 一种写法）。  
   验收：改 `web.rs` 后 rust-analyzer / tsc **不必先启动 HTTP** 就能补全；页面不再手写路径。

2. **表单不要两套世界**  
   `useForm` 发现 `File` 就走 `FormData`（CSRF 照带）；Rust `#[server]` / FormRequest 都能 `v.file_field(...)`；错误仍挂 `form.errors`。

3. **校验错误用稳定码**（已做）  
   `Rule` 返回 `username.taken`；`trans_error()` 与前端 `t()` 读 `lang/*.json`。`useForm.messages` 按码覆盖。

4. **Action 返回值进 TS**  
   `ActionOk<T>` 走与 `ViewData` 同一套 derive；`useForm<LoginOk>` 能补全 `redirect` / `access_token`。

### 先不做

真 SMTP、真 OAuth provider、Laravel `Http::` 出站门面、更重 DataTable、Redis、Toasty `create!` 糖、合并 `LoginUser` / `AuthUser` / `User`。那是运行时完整度或上游成本，不是写页面的手感。第三方 API 在业务包用 `reqwest` 调即可（[08 §7](./08-platform.md#7-出站-http-调第三方)）。

## P3：生产运行闭环

已完成：不可变版本目录、共享数据面、候选 PID 就绪验证、原子 current 切换、优雅排水、稳定生产配置及本地到服务器上传脚本；框架 `SessionStore`（memory / file / Redis 适配）、可配置会话/JWT 时长、`namix::Jwt` 与滚动更新前的共享会话预检。

下一项：

1. 真实 Redis 客户端集成（限流 + 会话）、数据库 Session Store，以及跨进程密码重置 token。
2. 真实 S3/邮件/通知驱动和可观测性 exporter。
3. 守护进程/容器编排适配、迁移 preflight、发布保留策略和 CI 远程部署凭据集成。
