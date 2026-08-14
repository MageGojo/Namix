# 模型

持久化用 **Toasty**。业务约定：

- `models/`：表结构、关联、简洁查询助手（≈ Eloquent）
- `services/`：注册、改资料、发帖等写操作与领域规则
- `seeders/`：演示/初始化数据
- `models/registry.rs`：所有模型必须登记，否则建不了表

---

## 设计原理

1. **Model 偏数据，Service 偏业务**：控制器不直接 `toasty::create!` 一长串。
2. **关联用属性宏**：`has_many` / `has_one` / `belongs_to` / M2M `via`。
3. **全局 Db**：`namix::db::{run, optional, vec}` 包一层，助手方法里不传来传去 `Db`。
4. **命名避开 Toasty 占用**：列表用 `User::list()`，不要指望 `User::all().await` 当「全部行」（`all()` 是查询构建器）。
5. **授权用库里的行，不用前端的归属字段**：更新/删除前 `Model::find`，再 `authorize(actor, policy, …, Some(&model))`。见 [授权](./07-authorization.md)。

---

## 1. 定义模型

```rust
//! app/src/models/user.rs

use namix::db;
use super::post::Post;
use super::profile::Profile;

#[derive(Clone, Debug, toasty::Model)]
#[table = "users"]
pub struct User {
    #[key]
    #[auto]
    pub id: u64,

    #[unique]
    pub username: String,

    /// 勿直接序列化给前端
    pub password_hash: String,

    pub name: String,
    pub is_vip: bool,
    pub email_verified_at: Option<jiff::Timestamp>,

    #[auto]
    pub created_at: jiff::Timestamp,
    #[auto]
    pub updated_at: jiff::Timestamp,

    #[has_many(pair = author)]
    pub posts: toasty::Deferred<Vec<Post>>,

    #[has_one(pair = user)]
    pub profile: toasty::Deferred<Option<Profile>>,
}
```

### 常见字段宏

| 宏 | 作用 |
|----|------|
| `#[key]` + `#[auto]` | 主键自增 |
| `#[unique]` | 唯一约束 |
| `#[auto]` on timestamps | 自动维护 |
| `#[has_many(pair = …)]` | 一对多 |
| `#[has_one(pair = …)]` | 一对一 |
| `#[belongs_to(key = user_id, references = id)]` | 多对一 |
| `#[has_many(via = post_tags.tag)]` | 多对多读取 |

文章侧示例：

```rust
#[derive(Clone, Debug, toasty::Model)]
#[table = "posts"]
pub struct Post {
    #[key]
    #[auto]
    pub id: u64,

    pub user_id: u64,
    pub title: String,
    pub body: String,

    #[belongs_to(key = user_id, references = id)]
    pub author: toasty::Deferred<User>,

    #[has_many(via = post_tags.tag)]
    pub tags: toasty::Deferred<Vec<Tag>>,
}
```

---

## 2. 查询助手（推荐写法）

```rust
impl User {
    pub async fn find(id: u64) -> Option<Self> {
        db::optional(move |mut db| async move { User::get_by_id(&mut db, id).await }).await
    }

    pub async fn find_by_username(username: impl Into<String>) -> Option<Self> {
        let username = username.into();
        db::optional(move |mut db| {
            let username = username.clone();
            async move { User::get_by_username(&mut db, username.as_str()).await }
        })
        .await
    }

    /// ≈ Laravel User::all()
    pub async fn list() -> Vec<Self> {
        db::vec(|mut db| async move { User::all().exec(&mut db).await }).await
    }

    pub async fn load_profile(&self) -> Option<Profile> {
        let user = self.clone();
        db::run(move |mut db| async move { user.profile().exec(&mut db).await })
            .await
            .ok()
            .flatten()
    }

    pub async fn load_posts(&self) -> Vec<Post> {
        let user = self.clone();
        db::vec(move |mut db| async move { user.posts().exec(&mut db).await }).await
    }
}
```

### `db` 助手语义

| API | 成功 | 失败 |
|-----|------|------|
| `db::run` | `Result<T>` | 原样返回 Err |
| `db::optional` | `Option<T>` | 当作 None |
| `db::vec` | `Vec<T>` | **打日志并返回空 Vec**（演示友好；关键写路径别依赖它吞错） |

---

## 3. 在控制器 / Service 里怎么用

### 读

```rust
let Some(db_user) = User::find(user.id).await else {
    return req.redirect_guest_to(route::main::login);
};
let posts = db_user.load_posts().await;
let profile = db_user.load_profile().await;
```

### 写（放 Service）

```rust
// services/user.rs：业务错误用 AppError，不要再用 String
pub async fn register(&self, username: &str, password: &str) -> Result<User, AppError> {
    if User::find_by_username(username).await.is_some() {
        return Err(AppError::validation("username", "username.taken"));
    }
    let password_hash = Self::hash_password(password)?;
    db::run(move |mut db| async move {
        toasty::create!(User {
            username: username.as_str(),
            password_hash: password_hash.as_str(),
            name: username.as_str(),
            is_vip: false,
            email_verified_at: None,
        })
        .exec(&mut db)
        .await
        // …
    })
    .await
    .map_err(AppError::internal)
}
```

控制器保持：

```rust
let user = UserService::new().register(&form.username, &form.password).await?;
```

发帖写路径还会先 `authorize`（见 [授权](./07-authorization.md)），再调用 `create_post` / `update_post` / `delete_post`。

调第三方 API 同样放 Service（`reqwest`），不要写在控制器或 TS 里。见 [平台 · 出站 HTTP](./08-platform.md#7-出站-http-调第三方)。

---

## 4. 注册到 schema

```rust
// models/registry.rs
pub fn model_set() -> namix::db::ModelSet {
    toasty::models!(
        User,
        Profile,
        Post,
        Tag,
        PostTag,
        Note,
        LoginLog,
        // 新模型必须加在这里
    )
}
```

`main.rs`：

```rust
Boot::new("main")
    .toml(include_str!("../namix.toml"))
    .models(app::models::registry::model_set())
    …
```

新文件会被 build 脚本 `mod`，但**不进 `model_set` 就不会建表**。

---

## 5. 迁移与种子

```bash
nx migrate generate   # 按 Model 生成 SQL
nx migrate apply
nx seed               # 跑 seeders
```

种子示例逻辑（幂等）：

```rust
if !User::list().await.is_empty() {
    return Ok(());  // 已有数据则跳过
}
// 创建 alice / bob、profile、posts、tags…
```

开发库路径默认：`sqlite:./storage/namix.db`（相对运行根目录）。
生产热更新共享库：`dist/data/storage/namix.db`（见部署相关能力）。

---

## 6. Laravel 对照

| Laravel | Namix |
|---------|--------|
| `User::find(1)` | `User::find(1).await` |
| `User::where('username', $u)->first()` | `User::find_by_username(u).await` |
| `User::all()` | `User::list().await` |
| `$user->posts` | `user.load_posts().await` |
| `$user->profile` | `user.load_profile().await` |
| `$hidden = ['password']` | 字段名 `password_hash` + 勿放进 ViewData |
| SoftDeletes / $fillable / Factory | 未作为一等能力；用 Service / Seeder |

---

## 7. 新建模型清单

```bash
nx make model Article -m    # 可同时生成迁移
```

1. 补字段与关联宏
2. 写 `find` / `list` / `load_*` 助手（按需）
3. **加入 `registry::model_set`**
4. `nx migrate generate && nx migrate apply`
5. 写操作进对应 Service
6. 需要演示数据时加 Seeder

---

## 易错点

| 问题 | 正确做法 |
|------|----------|
| 新 Model 没进 registry | 表不存在 / push_schema 对不上 |
| 把 `password_hash` 放进 `ViewData` | 只传 `username` 等安全字段 |
| 用 `db::vec` 做转账式写路径 | 改用 `db::run` 并处理 `Result` |
| 在控制器里复制关联加载 | 收到 Model 助手或 Service |
| 以为改 Model 自动改生产库 | 要走迁移；热更新不会魔法迁库 |