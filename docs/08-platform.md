# 平台能力：资源路由 · 分页 · Cache · Queue · Storage · 测试

框架已提供 Laravel 风格的平台 API（内存/本地驱动可直接跑）。业务侧按需接入；开关与目录见 [FEATURES.md](./FEATURES.md)。

---

## 1. 资源路由

见 [路由 · resource](./02-routes.md#4-资源路由-resource)。`nx make resource Posts` 生成控制器骨架；`resource("posts", PostsController)` 注册七件套命名路由。

示例 `app` 的文章页用手写 POST 表单路径，便于 CSRF；REST API 可改用 PATCH/DELETE。

---

## 2. 分页与查询白名单

```rust
use namix::prelude::*;

let sort = SortWhitelist::new(["created_at", "title"]);
let query = QueryOptions::from_request(&req, &sort, ["status"])?;
// query.page / per_page / sort / filters

let page = Paginator::from_items(all_posts, &query);
// page.data, total, current_page, last_page, from, to
```

要点：

- **排序字段必须白名单**，禁止把用户输入直接拼进 `ORDER BY`。
- `per_page` 有上限（见 `QueryOptions::MAX_PER_PAGE`）。
- `Paginator` 可 `serde` 进页面 props 或 JSON API。

---

## 3. Cache

```rust
use namix::prelude::*;
use std::time::Duration;

let cache = Cache::new(MemoryCache::default());
cache.put("stats:users", &count, Some(Duration::from_secs(60)))?;
let hit: Option<u64> = cache.get("stats:users")?;
let value = cache.remember("expensive", Some(Duration::from_secs(30)), || compute())?;
cache.forget("stats:users")?;
cache.flush()?;
```

- 未命中是 `Ok(None)`；后端故障是 `Err(CacheError)`，再映射为 `AppError::internal`。
- 生产可换 Redis 适配（应用接入驱动）；开发默认内存即可。

---

## 4. Queue / Job

```rust
use namix::prelude::*;
use anyhow::Context;

struct WelcomeMail { to: String }
impl Job for WelcomeMail {
    fn name(&self) -> &'static str { "welcome_mail" }
    fn handle(self: Box<Self>) -> JobFuture {
        Box::pin(async move {
            Mail::send(MailMessage::new(self.to, "Welcome").text("hi"))
                .context("welcome mail")?;
            Ok(())
        })
    }
}

let q = Queue::memory(64);
q.dispatch(WelcomeMail { to: "a@b.c".into() }).await?;
let _worker = q.clone().worker(); // 或 work_once()
```

- Job 边界用 `anyhow::Result`，便于 `.context(...)`；worker 记完整错误链。
- 骨架：`nx make job WelcomeMail`。
- HTTP/业务域仍返回具体 `AppError`，不要把 `anyhow` 透出控制器。

重启不丢活用 **durable queue**（不要 Redis）：

```toml
[queue]
driver = "file"            # memory | file | sqlite
path = "./storage/queue"
```

```rust
#[derive(Serialize, Deserialize)]
struct WelcomePing { pub email: String }
impl QueuedJob for WelcomePing {
    const NAME: &'static str = "welcome_ping";
    fn handle(self) -> JobFuture { Box::pin(async move { Ok(()) }) }
}

register_job::<WelcomePing>();
dispatch_job_later(WelcomePing { email: "a@b.c".into() }, Duration::from_secs(5))?;
```

`nx work` → `cargo run -p app --bin work` 循环消费。HTTP 进程不自动跑 worker。

---

## 5. Storage

命名磁盘写在 `namix.toml` `[storage]`（空配置时 Boot 安装 `local` + `public`）。签名密钥从 `session_secret` / `NAMIX_SESSION_SECRET` 派生，重启后临时 URL 仍然有效。

```toml
[storage]
default = "local"

[storage.disks.local]
driver = "local"
root = "./storage/app"
url = "/storage/private"
visibility = "private"

[storage.disks.public]
driver = "local"
root = "./storage/app/public"
url = "/storage"
visibility = "public"

[storage.links]
"public/storage" = "storage/app/public"
```

```rust
use namix::prelude::*;
use std::time::Duration;

let storage = Storage::disk("local")?;           // 或 Storage::default_disk()?
storage.put("avatars/a.png", bytes)?;
storage.put_with_policy("avatars/a.png", bytes, &policy)?;
storage.exists("avatars/a.png")?;
storage.copy("avatars/a.png", "avatars/b.png")?;
storage.files("avatars")?;                       // 一层；all_files 递归
let key = storage.put_file("avatars", &uploaded)?;
storage.set_visibility(&key, Visibility::Private)?;
let signed = storage.temporary_url(&key, Duration::from_secs(300))?;
let upload = storage.temporary_upload_url(&key, Duration::from_secs(120))?;
let public = Storage::disk("public")?;
public.url("logo.png");                          // → /storage/logo.png
```

测试用内存盘：

```rust
let photos = Storage::fake("photos");
photos.put("a.png", b"ok")?;
photos.assert_exists("a.png");
```

图片（本地 `image` crate，png/jpeg/gif/webp）：

```rust
storage.image("shot.png")?.cover(400, 400)?.to_webp(80)?.save()?;
```

- `StorageError`：策略/键/只读 → 4xx；I/O → 带 source 的 500（见 [ERRORS.md](./ERRORS.md)）。
- 公开文件：`GET /storage/*path` 读 `public` disk；私有 disk 的 GET/PUT 必须带 HMAC `expires`+`signature`（签名 PUT 免 CSRF）。
- `nx storage link` / `unlink`：按 `[storage.links]` 建符号链接（默认 `public/storage` → `storage/app/public`）。
- S3 / FTP / SFTP **不内置协议 crate**。写 `Storage::extend("s3", |cfg| { … })` 再在 toml 里 `driver = "s3"`。S3 仍可用 `S3CompatibleStorage<T: S3Transport>`。
- 包装：`storage.scoped("avatars")?`、`storage.read_only()`，或 toml `driver = "scoped"|"readonly"`。
- `UploadedFile` 没有 `.store()`：用 `Storage::put_file` / `put_file_as`（`namix-http` 不反向依赖门面）。

---

## 6. TestClient

进程内测路由、Cookie、表单、Action、WS，无需真开端口：

```rust
use namix::prelude::*;

let mut client = TestClient::new(web::routes())
    .with_same_origin("http://127.0.0.1:3000")?;
let res = client.get("/login").await;
assert!(res.is_success());

// 经典表单：自动带 CSRF（需先打开会种 cookie 的页面或 fetch_csrf）
let res = client
    .csrf_form(
        Method::POST,
        "/posts",
        [("title", "hi"), ("body", "world")],
    )
    .await;
```

覆盖面以框架单测与业务夹具为准；迁移/临时库夹具见 `NEXT.md` P2。

---

## 7. 出站 HTTP（调第三方）

框架 **没有** Laravel `Http::get` 门面。在 **服务器进程**里调外部 API：业务包加 `reqwest`，写在 `services/`，控制器 / `#[server]` 只 `await` Service。

```toml
# app/Cargo.toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

```rust
let body: serde_json::Value = reqwest::Client::new()
    .get("https://api.example.com/v1/foo")
    .bearer_auth(&std::env::var("FOO_TOKEN").map_err(AppError::internal)?)
    .timeout(Duration::from_secs(8))
    .send()
    .await
    .map_err(AppError::internal)?
    .error_for_status()
    .map_err(AppError::internal)?
    .json()
    .await
    .map_err(AppError::internal)?;
```

| 要点 | |
|------|--|
| 密钥 | 环境变量，不要写进仓库 `namix.toml` |
| 错误 | `AppError::internal`；浏览器只见通用 500 |
| 返回给前台 | 只映射展示字段，见 [控制器 · 在 Server Action 里调第三方](./01-controllers.md#在-server-action-里调第三方) |
| 慢 / 可失败 | `QueuedJob` + `nx work`，别堵在这次请求上 |
| 邮件 / 短信网关 | 走 `Mail` / `Sms` 的 `register_transport`，HTTP 写在 transport 里 |

不要在 TS 里 `fetch('https://api.example.com')` 带 Key。等出现多个真实出站且需要 `Http::fake()` 时再考虑框架门面；见 [NEXT.md](./NEXT.md)「先不做」。

---

## 骨架命令

```bash
nx make page Notes
nx make error
nx make resource Posts
nx make policy Post
nx make job SendDigest
nx make mail Welcome
nx make notification InvoicePaid
nx make test posts_update_forbidden
nx storage link          # public/storage → storage/app/public
nx clean                 # 删 target / node_modules / public/build
```
