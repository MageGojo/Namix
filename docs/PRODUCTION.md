# 生产发布与无感更新

Namix 的生产发布使用**不可变版本目录 + 共享数据面 + 候选进程就绪后原子切换**。应用代码、前端静态资源和运行配置互不覆盖：改 Bug 时只上传新版本，不直接覆盖线上 `current`。

## 先决条件

- Linux 单机；`SO_REUSEPORT` 用于新旧进程重叠接流。
- release 后端是宿主原生二进制，构建机必须与生产机的 OS 和 CPU 架构一致。部署 Linux `x86_64` 时，请在 Linux `x86_64` 构建机、CI runner 或容器内执行 `nx build`，不要直接上传 macOS 构建产物。
- 公网 TLS 由 Caddy、Nginx 或云负载均衡终止；Namix 只监听 `127.0.0.1:3000`。
- 若使用 PostgreSQL/MySQL/Redis，已启用对应 Namix Cargo feature 且依赖独立运行；密钥只放服务器环境文件，权限为 `600`。
- 数据库迁移采用 expand → deploy → contract：候选版本和旧版本在排水窗口内必须能同时读写 schema。
- 会话驱动见 `[session]`：`memory` 仅适合单进程开发；生产滚动更新要求 `file`（经 `dist/data/storage` 共享）或 `redis`（应用接入 `RedisSessionStore`）。`lifetime_secs` / `jwt_lifetime_secs` 分别控制 Cookie 与 Bearer JWT；`nx update` 在存在旧进程时会预检；进程内会话需维护窗口，或显式设置 `NAMIX_ALLOW_MEMORY_SESSIONS=1`。
- 反代后取真实客户端 IP：在 `[security] trusted_proxies` 填入代理 CIDR/地址（见 `ops/production/namix.toml.example`）。框架用 `TrustedProxies` 解析 `X-Forwarded-For`；只信任名单内的 peer。边缘仍应做一层限流，勿把未校验的转发头直接暴露给公网。

## 一次性服务器准备

```text
/srv/APP_A/
├── dist/
│   ├── data/
│   │   ├── namix.toml        # 稳定生产配置
│   │   ├── logs/             # detached 候选进程 stdout/stderr
│   │   └── storage/
│   │       └── action_seal.key # 共享 Action 私钥，0600
│   ├── 1.0.0/                # 不可变发布包
│   └── current -> 1.0.0      # 唯一活动版本指针
└── ...                       # nx 命令与项目元数据
```

1. 将 [`ops/production/namix.toml.example`](../ops/production/namix.toml.example) 保存为 `/srv/APP_A/dist/data/namix.toml`，填写实际域名、数据库，并确认 `[session] driver = "file"`（或已接入的 `redis`）以及合适的 `lifetime_secs` / `jwt_lifetime_secs`。
2. 在服务环境中设置 `NAMIX_SESSION_SECRET`（至少 32 个高熵字符）和 `DATABASE_URL`。`nx start` / `nx update` 要求共享配置文件存在，并固定注入 `NAMIX_CONFIG=/srv/APP_A/dist/data/namix.toml`、`NAMIX_ENV=production` 和 `NAMIX_VITE_DEV=0`；生产进程不会回落到发布包中的开发配置。
3. 在受保护且匹配生产平台的构建环境执行首次 `nx build --ver 1.0.0`，再将生成的 `dist/data/storage/action_seal.key` 通过密钥管理通道安装到服务器同一路径并设置 `0600`。该文件可为旧版 32-byte X25519 secret 或 64-byte `secret || public`，必须跨版本固定；发布包和 `MANIFEST.json` 只含公钥。后续切勿单独替换服务器私钥。
4. 将 [`ops/production/Caddyfile.example`](../ops/production/Caddyfile.example) 配到 TLS 边缘，并让防火墙只开放 80/443。
5. 上传 `dist/1.0.0/` 后执行 `nx update --ver 1.0.0 --port 3000`；也可直接使用下文部署脚本完成首次启动。

`GET /__namix/health` 返回版本与就绪状态，可供负载均衡探测。`GET /__namix/routes` 仅用于调试，不作为发布健康信号。

## 日常更新：匹配生产平台的构建机到服务器

在与生产机 OS/CPU 架构一致的构建环境中通过测试后，按版本发布：

```bash
cargo test -p namix -p namix-http --all-features
(cd app && npm run typecheck && npm run build)
nx build --ver 1.0.1

export NAMIX_DEPLOY_HOST='deploy@HOST'
export NAMIX_DEPLOY_ROOT='/srv/APP_A'
export NAMIX_DEPLOY_PORT=3000
ops/deploy-release.sh 1.0.1
```

完整 release 的固定构建顺序是 Rust codegen/check → 前端 → release 后端，确保 Vite 使用本次 Rust 宏生成的 Action 与页面类型。`nx build --frontend-only` 只刷新 `app/public/build`，不会创建版本目录，也不会切换 `dist/current`。

构建环境若已有 `dist/data/storage/action_seal.key`，`nx build` 会从私钥推导公钥，并通过 `NAMIX_ACTION_SEAL_PUBLIC_KEY` 固定本次前端/WASM 构建；`app/storage` 中的旧 key 不会覆盖共享 key。首次构建会将应用 key 写入共享数据面。生成的 `MANIFEST.json` 仅记录 `action_seal_public_key`，不记录 secret。

`nx build` 会在 release 的 `MANIFEST.json` 记录构建宿主的 Rust target、OS 和 CPU 架构。`nx start` 与 `nx update` 在创建候选进程前校验这些字段；例如把 macOS `aarch64` 构建物上传到 Linux 后，会报告 release/runtime 平台不兼容，而不是等到执行二进制时失败。旧版清单若缺少 `target`、`os` 或 `arch`，需在目标平台重新执行 `nx build`。

启动和切换前还会校验 release 的 `MANIFEST.json`、可执行文件、Vite manifest 及其引用资源；版本必须是 `x.y.z`。预检会从服务器 `dist/data/storage/action_seal.key` 推导公钥并与清单比较，私钥缺失、格式错误或公钥漂移都会在创建候选进程前终止。每个版本的 `storage` 也必须准确指向 `dist/data/storage`，真实目录或错误链接会让启动提前终止。生产数据库由迁移/显式初始化流程创建，`nx start` / `nx update` 不会从 `app/storage/namix.db` 自动播种。

脚本先上传到 `dist/.incoming-1.0.1/`，校验传输完成后才在服务器上改名为 `dist/1.0.1/`。随后服务器执行 `nx update --ver 1.0.1`：

1. 预检 release 平台、Action seal 公钥和共享数据面；若已有旧进程，还会检查 `[session]` 是否为共享驱动（`file` / `redis`）。
2. 新进程的 stdout/stderr 追加写入 `dist/data/logs/<version>.log`（目录 `0700`、文件 `0600`），随后在同端口绑定完成后写入仅发布器可见的 ready 标记；这证明的是**新 PID**，不会误把旧进程的 HTTP 200 当成成功。
3. 发布器原子替换 `dist/current`。
4. 旧进程收到 `SIGTERM`，停止接收新连接并最多排水 20 秒；超时才强制结束。
5. 新进程未就绪或切换失败时，发布器停止候选进程、清理候选 pidfile/ready 标记，并让旧 PID 和 `current` 保持原样；报错会附带候选日志路径。

所有静态资源位于版本目录；旧目录保留，因此已经打开页面引用的带哈希资源不会在切换时 404。

`nx build` 成功只说明版本目录写出来了，**不能**当成浏览器能加载 JS。HTML 写死 `/build/…`、反代只转发 `/lr*`、Vite `base` 与运行时前缀不一致、或进程找不到 `public/build` 时，页面 200 但 Island 不水合。本地用仓库脚本在 `/tmp` 里拉真实 HTML，并把 JS/CSS/WASM 全部 GET 一遍（含子路径 `/lr`）：

```bash
ops/smoke-nx.sh
```

脚本会 `nx new` 一份 lean 脚手架做编译自检，再搭假 `dist/0.0.0-smoke` 起示例应用（不覆盖仓库已有 `dist/<semver>`，也不部署）。`KEEP_WORKDIR=1` 可保留临时目录；`SKIP_NEW=1` / `SKIP_RELEASE=1` 可只跑其中一轨。默认不进 GitHub Actions（完整 `nx new` + npm + 起进程过重）；需要时再加 nightly。

## 回滚与清理

```bash
# 只在确认目标版本可运行后切换指针；不会重启进程
nx update --ver 1.0.0 --swap-only

# 用 1.0.0 启动一个候选进程并排水当前版本
nx update --ver 1.0.0 --port 3000

nx status
```

保留当前版本、上一个版本和至少一个经过验证的回滚版本。只删除既非 `current`、也不在排水中的目录。不要覆盖 `dist/data/`。
