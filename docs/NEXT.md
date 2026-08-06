# 下一阶段路线图

以下按高质量 Rust 全栈框架的收益与风险排序。每项应伴随 API 文档、单元测试和至少一个示例应用用例。

## P0：安全与运行可靠性（已完成第一版）

- CSRF/Origin：浏览器 mutation 强制同源 Origin + double-submit token；Bearer-only API 自动豁免。
- 限流：提供 IP / 已认证用户策略，Action 预设 login、registration、action 三档，上传单独限流。
- 错误边界：`AppError` 映射 HTML、JSON、Action，并保留 `Retry-After`。
- 配置启动校验：生产 HTTPS、会话密钥、Action seal、迁移策略和数据库配置必须一致。
- 会话：CSPRNG 签名 token、过期、轮换、全设备登出、一次性密码重置、旧 SHA-256 登录后升级 Argon2id。

后续增强：可信代理 CIDR 解析、Redis 限流/会话驱动、上传 body 与磁盘配额、跨进程密码重置 token。

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

## P3：生产运行闭环

已完成：不可变版本目录、共享数据面、候选 PID 就绪验证、原子 current 切换、优雅排水、稳定生产配置及本地到服务器上传脚本。

下一项：

1. Redis/数据库 Session Store，并在滚动更新前强制检查共享会话配置。
2. Redis 限流、真实 S3/邮件/通知驱动和可观测性 exporter。
3. 守护进程/容器编排适配、迁移 preflight、发布保留策略和 CI 远程部署凭据集成。
