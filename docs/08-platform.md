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

---

## 5. Storage

```rust
use namix::prelude::*;

let storage = Storage::new(LocalStorage::new("./storage/app", "/files"));
storage.put("avatars/a.png", bytes)?;
// UploadPolicy：限制扩展名 / 最大字节等，违规 → 4xx
storage.put_with_policy("avatars/a.png", bytes, &policy)?;
let url = storage.url("avatars/a.png");
let signed = storage.temporary_url("avatars/a.png", Duration::from_secs(300))?;
storage.verify_temporary_url(/* key, expires, signature */)?;
storage.delete("avatars/a.png")?;
```

- `StorageError`：策略违规 → 4xx；I/O → 带 source 的 500（见 [ERRORS.md](./ERRORS.md)）。
- S3 等远程驱动可替换 `StorageDriver`；开发用本地目录。

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

## 骨架命令

```bash
nx make resource Posts
nx make policy Post
nx make job SendDigest
nx make mail Welcome
nx make notification InvoicePaid
nx make test posts_update_forbidden
```
