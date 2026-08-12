# Features 开关完整手册

`nx new` **默认 lean**：只有 **controllers**、**routes**、**middleware**，以及 **views**（`[features].pages = true`）。  
其余能力全部关掉，用 `namix.toml` 与 `Cargo.toml` 按需打开。仓库内示例 `app/` 为完整业务面，可对照本页逐项对照。

本文覆盖全部可文档化的开关：目录 feature、Cargo feature、数据库、会话、邮件、短信、安全与 Action 密封。不含「以后再写」的占位描述。

---

## 1. 两层开关怎么配合

| 层 | 文件 | 作用 |
|----|------|------|
| 目录 / 构建 | `app/namix.toml` → `[features]` | `namix-build` 创建或删除带 `.namix-feature` 的目录，并生成 `namix_modules.rs` / 视图注册表 |
| 编译体积 | `app/Cargo.toml` → `namix = { features = [...] }` | 编进 Toasty 驱动、`req.view` 等可选代码 |
| 运行时 | `namix.toml` 其它段（`[database]` / `[session]` / `[mail]` / `[sms]` / `[security]`） | Boot 是否连库、会话驱动、邮件/短信驱动、生产约束 |

规则：

1. 关掉 `[features].models` 后，带标记的 `src/models/` 会在下次 `cargo build` 时被删掉。
2. 手写目录若**没有** `.namix-feature`，build 脚本**不会**删除（保护存量代码）。
3. `nx make model|validator` 会写入对应标记，并打印需打开的配置项；仍须你手动改 `namix.toml` / Cargo。

---

## 2. `namix.toml [features]`（目录）

默认脚手架（`nx new`）取值：

```toml
[features]
models = false
services = false
validators = false
requests = false
pages = true          # → src/views/
events = false
listeners = false
seeders = false
action_seal = true
```

| 键 | 默认（new） | 目录 | 打开后做什么 |
|----|-------------|------|----------------|
| `models` | `false` | 单：`src/models/`；多：`src/common/models/` | Toasty 实体 + `registry.rs` 的 `model_set()`；`Boot::models(...)` / migrate / seed 需要 |
| `services` | `false` | 单：`src/services/`；多：`src/common/services/` | 写库与领域服务；与 models / database 一起用 |
| `validators` | `false` | 单：`src/validators/`；多：`common/` 或端目录 | `FormField` / 规则校验；`nx make validator` |
| `requests` | `false` | `src/requests/`（或 common） | 可选请求 DTO 分层；多数项目用 validators 即可 |
| `pages` | `true` | `src/views/` | React 页面与生成物；配合 Cargo `pages` 才能用 `req.view`。工具链在 **`app/package.json`**（Vite），**不要**再建根目录独立 `frontend/` SPA |
| `events` | `false` | `src/events/` 或 `common/events/` | 领域事件类型；`dispatch(...)` |
| `listeners` | `false` | `src/listeners/` 或 `common/listeners/` | `listen(...)`；多应用常在 bin 里 `register::all()` |
| `seeders` | `false` | `src/seeders/` 或 `common/seeders/` | 种子数据；需要 `[[bin]] seed` + `nx seed` |
| `action_seal` | `true` | （无目录） | `#[server]` 传输密封：生产整包加密；开发可 `false` 看明文 JSON。可用环境变量 `NAMIX_ACTION_SEAL=0\|1` 覆盖；改开关后执行 `npm run build:wasm` |

核心目录**永远**存在（不靠 feature）：

- 单应用：`src/controllers/`、`src/routes/`、`src/middleware/`
- 多应用：各端同上；`src/common/middleware/` 始终存在

---

## 3. Cargo features（`namix` 依赖）

写在业务包 `app/Cargo.toml`：

```toml
[dependencies]
namix = { workspace = true, features = ["pages"] }   # lean 默认
# 连库示例：
# namix = { workspace = true, features = ["pages", "sqlite"] }
# 也可直接依赖 toasty，并与 driver 对齐：
# toasty = { version = "0.9", default-features = false, features = ["sqlite", "serde"] }
```

| Feature | 作用 |
|---------|------|
| （默认空） | 最小门面：HTTP、Boot、校验、会话、邮件/短信门面；**不含** Toasty |
| `pages` | 启用 `namix-http/pages`：`req.view` / ViewData SSR 路径 |
| `models` | 透传 `namix-http/models`（标记位；实体仍靠 Toasty + 你的 `src/models`） |
| `services` | 透传 `namix-http/services` |
| `requests` | 透传 `namix-http/requests` |
| `sqlite` | Toasty + SQLite（`db` 是它的别名） |
| `mysql` | Toasty + MySQL |
| `postgresql` | Toasty + PostgreSQL |
| `turso` | Toasty + Turso |
| `dynamodb` | Toasty + DynamoDB |
| `db` | 等同 `sqlite` |

lean 脚手架只开 `pages`。打开数据库时至少再加一个驱动 feature，并设 `[database] enabled = true`。

---

## 4. 数据库 `[database]`

```toml
[database]
enabled = false                 # lean 默认；true 时 Boot 连接
driver = "sqlite"               # sqlite | mysql | postgresql | custom
url = "sqlite:./storage/namix.db"
# host / port / name / username / password 可拼 URL
push_schema = true              # 开发可 true；生产请 false，改用 migrate
```

环境变量 `DATABASE_URL` 可覆盖解析后的连接串。

打开清单：

1. `[database] enabled = true`
2. Cargo：`features = [..., "sqlite"]`（或其它驱动）
3. `[features] models = true`（以及通常 `services = true`）
4. `src/models/registry.rs` 提供 `model_set()`
5. `Boot::… .models(app::models::registry::model_set())`（多应用：`common::models::…`）
6. `Toasty.toml` + `database/`；`[[bin]]` `toasty` / `seed`（可从仓库 `app/` 复制）
7. `nx migrate generate|apply`；`nx seed`（需 `seeders = true`）

`nx doctor` 在 `enabled = false` 时不强制 registry / toasty / seed bin。

---

## 5. 会话 `[session]`

```toml
[session]
driver = "memory"                 # memory | file | redis
path = "./storage/sessions"       # file 驱动根目录（生产滚动更新共享 dist/data）
lifetime_secs = 604800            # Cookie / opaque 会话（秒）
jwt_lifetime_secs = 3600          # Bearer JWT（秒）
```

| 驱动 | 用途 |
|------|------|
| `memory` | 单进程开发默认；生产滚动更新禁止（除非 `NAMIX_ALLOW_MEMORY_SESSIONS=1`） |
| `file` | 多进程 / `nx update` 滚动：会话放共享数据平面 |
| `redis` | 应用接入 `RedisSessionStore` |

登录会发 opaque Cookie，并可返回 HS256 JWT（claims 含 `sid`，可与 Cookie 一并撤销）。详见 [PRODUCTION.md](./PRODUCTION.md)、[07-authorization.md](./07-authorization.md)。

---

## 6. 邮件 `[mail]` / 短信 `[sms]`

```toml
[mail]
driver = "log"                    # log | file |（可扩展 smtp）
from = "noreply@namix.local"
store = "./storage/mail"

[sms]
driver = "log"
log_otp = true                    # 开发：OTP 打到日志
```

| 门面 | 用法 |
|------|------|
| `Mail` | 发信；`log`/`file` 写 `store` 并打日志；示例页 `/mailbox`；入站 `POST /webhooks/mail/inbound` |
| `Sms` | OTP / 短信；`log_otp=true` 时验证码进日志 |

`nx make mail Name` / `nx make notification Name` 只生成骨架文件，不新增 `[features]` 键。

---

## 7. 安全 `[security]`

```toml
[security]
environment = "development"       # production 时强制更严校验
csrf = true
# session_secret = "..."          # 或环境变量 NAMIX_SESSION_SECRET
# tls_terminated_by_proxy = false
```

生产环境会要求 HTTPS（或显式 `tls_terminated_by_proxy = true`）、CSRF、Action 密封、禁用启动时 `push_schema`，并要求会话密钥。详见 [SECURITY_SCOPE.md](./SECURITY_SCOPE.md)、[PRODUCTION.md](./PRODUCTION.md)。

---

## 8. 按场景打开（checklist）

### A. 只要静态/HTML 首页（已是默认）

无需改配置。控制器用 `html(...)` / `text(...)`。

### B. React `req.view`

1. 保持 `[features].pages = true` 与 Cargo `pages`
2. 在 `src/views/pages/` 写页面；`npm run build`（或 `nx dev`）产出客户端 bundle（**运行时不依赖 Node SSR**）
3. 控制器：`req.view("home").data(...).island().render()`（壳与 props 由 Rust 内联，见 `docs/SSR-RUST.md`）

### C. 表单校验

1. `[features].validators = true`
2. `nx make validator Login`
3. 路由里用提取器或 `LoginForm::validate(&req)`

### D. 数据库 CRUD

按第 4 节清单；然后：

```bash
nx make model Article -m
nx migrate apply
# 可选
# namix.toml: services = true, seeders = true
nx seed
```

### E. 事件总线

1. `events = true`、`listeners = true`
2. 定义事件类型与 `listen`；在入口调用注册函数
3. 控制器 `dispatch(MyEvent { ... })`

### F. 多应用

`nx new demo --multi` 同样 lean：三端各有 controllers/routes，共享 `src/views/`（pages）与 `common/middleware`。  
打开 models 时落在 `src/common/models/`；端专属校验器：`nx make validator Checkout --app user`。

---

## 9. `nx` 命令与 feature 关系

| 命令 | lean 下能否用 | 额外条件 |
|------|---------------|----------|
| `nx new` | 是 | 生成 lean 项目 |
| `nx make controller\|resource\|policy\|job\|mail\|notification\|test` | 是 | 骨架目录，多数不入 `[features]` |
| `nx make validator` | 是（写文件） | 须 `validators = true`，否则下次 sync 可能删掉带标记目录 |
| `nx make model` | 是（写文件） | 须 `models = true` + database + Cargo 驱动 |
| `nx migrate *` | 否（缺 bin） | `toasty` bin + DB + models |
| `nx seed` | 否（缺 bin） | `seed` bin + `seeders = true` + DB |
| `nx export routes` | 是 | 需先跑过后端写出 `storage/routes.*` |
| `nx doctor` | 是 | lean 不强制 DB/registry；`--check` 跑 `cargo check` |
| `nx dev` / `build` / `start` / `update` / `stop` / `status` | 是 | 生产滚动更新需共享 session（file/redis） |
| `nx completion` | 是 | 与 feature 无关 |

---

## 10. 与示例 `app/` 的差异

示例 `app/namix.toml` 打开了完整业务面（`models/services/validators/pages/events/listeners/seeders = true`，`database.enabled = true`），并带有 toasty/seed bin。  
新项目用 lean；需要同等能力时按本页 checklist 逐项打开，或对照 `app/` 复制配置与 bin 模板。
