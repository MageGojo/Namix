# 生产发布与无感更新

Namix 的生产发布使用**不可变版本目录 + 共享数据面 + 候选进程就绪后原子切换**。应用代码、前端静态资源和运行配置互不覆盖：改 Bug 时只上传新版本，不直接覆盖线上 `current`。

## 先决条件

- Linux 单机；`SO_REUSEPORT` 用于新旧进程重叠接流。
- 公网 TLS 由 Caddy、Nginx 或云负载均衡终止；Namix 只监听 `127.0.0.1:3000`。
- 若使用 PostgreSQL/MySQL/Redis，已启用对应 Namix Cargo feature 且依赖独立运行；密钥只放服务器环境文件，权限为 `600`。
- 数据库迁移采用 expand → deploy → contract：候选版本和旧版本在排水窗口内必须能同时读写 schema。
- 当前示例应用的会话存储是进程内实现。要让已登录用户在新旧进程交叠时保持会话，生产接入共享 Session Store（Redis 或数据库）后再启用滚动更新；否则使用维护窗口发布。
- `trusted_proxies` 的 CIDR/X-Forwarded-For 解析仍在路线图中。TLS 反代期间，在 Caddy/边缘同时执行按真实客户端 IP 的限流，避免 Namix 仅看见代理地址。

## 一次性服务器准备

```text
/srv/APP_A/
├── dist/
│   ├── data/
│   │   ├── namix.toml        # 稳定生产配置
│   │   └── storage/          # SQLite、Action key、邮件等共享数据
│   ├── 1.0.0/                # 不可变发布包
│   └── current -> 1.0.0      # 唯一活动版本指针
└── ...                       # nx 命令与项目元数据
```

1. 将 [`ops/production/namix.toml.example`](../ops/production/namix.toml.example) 保存为 `/srv/APP_A/dist/data/namix.toml`，填写实际域名和数据库。
2. 在服务环境中设置 `NAMIX_SESSION_SECRET`（至少 32 个高熵字符）和 `DATABASE_URL`。发布包启动器发现共享配置后会自动设置 `NAMIX_CONFIG=/srv/APP_A/dist/data/namix.toml`。
3. 将 [`ops/production/Caddyfile.example`](../ops/production/Caddyfile.example) 配到 TLS 边缘，并让防火墙只开放 80/443。
4. 首次构建并启动：`nx build --ver 1.0.0 && nx start --port 3000`。

`GET /__namix/health` 返回版本与就绪状态，可供负载均衡探测。`GET /__namix/routes` 仅用于调试，不作为发布健康信号。

## 日常更新：本地到服务器

本地通过测试后，按版本发布：

```bash
cargo test -p namix -p namix-http --all-features
(cd app && npm run typecheck && npm run build)
nx build --ver 1.0.1

export NAMIX_DEPLOY_HOST='deploy@HOST'
export NAMIX_DEPLOY_ROOT='/srv/APP_A'
export NAMIX_DEPLOY_PORT=3000
ops/deploy-release.sh 1.0.1
```

脚本先上传到 `dist/.incoming-1.0.1/`，校验传输完成后才在服务器上改名为 `dist/1.0.1/`。随后服务器执行 `nx update --ver 1.0.1`：

1. 新进程在同端口绑定完成后写入仅发布器可见的 ready 标记；这证明的是**新 PID**，不会误把旧进程的 HTTP 200 当成成功。
2. 发布器原子替换 `dist/current`。
3. 旧进程收到 `SIGTERM`，停止接收新连接并最多排水 15 秒；超时才强制结束。
4. 新进程未就绪或切换失败时，旧 PID 和 `current` 保持原样。

所有静态资源位于版本目录；旧目录保留，因此已经打开页面引用的带哈希资源不会在切换时 404。

## 回滚与清理

```bash
# 只在确认目标版本可运行后切换指针；不会重启进程
nx update --ver 1.0.0 --swap-only

# 用 1.0.0 启动一个候选进程并排水当前版本
nx update --ver 1.0.0 --port 3000

nx status
```

保留当前版本、上一个版本和至少一个经过验证的回滚版本。只删除既非 `current`、也不在排水中的目录。不要覆盖 `dist/data/`。
