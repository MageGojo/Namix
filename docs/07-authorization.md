# 授权（Policy / Gate）

对应 Laravel 的 `$this->authorize('update', $post)`：**前端只提供资源 ID（和要改的内容），真正拿来比对的是「会话里的当前用户」与「数据库查出的资源归属」**。不要信表单/JSON 里自称的 `user_id`、`is_admin`。

相关能力：

| 能力 | 用途 |
|------|------|
| `require_login` / `require_guest` / `require_vip` | 路由门禁（过 / 不过） |
| `AuthView` | 页面渲染时服务端分支，**不下发**授权字段到 props |
| `Policy` + `authorize` / `Gate` | 写操作：会话身份 vs **库里的**资源 |

零授权 props 约定见 [控制器 · 零授权 props](./01-controllers.md)与 [`SECURITY_SCOPE.md`](./SECURITY_SCOPE.md)。

本仓库已落地：

| 文件 | 作用 |
|------|------|
| [`app/src/policies/post_policy.rs`](../app/src/policies/post_policy.rs) | `Policy<LoginUser, Post>` |
| [`app/src/controllers/posts.rs`](../app/src/controllers/posts.rs) | `create` / `update` / `destroy` 调用 `authorize` |
| `nx make policy Post` | 生成骨架（目录存在即编进 `namix_modules`，无需 `[features]`） |

---

## 设计原理（在比什么）

```text
前端提交                服务端判定
─────────               ─────────────────────────────
post_id, title, body  →  1. AuthUser / LoginUser ← 会话 Store（可信）
                         2. Post::find(post_id) ← 数据库（可信）
                         3. authorize(actor, Policy, Update, Some(&post))
                              典型：post.user_id == actor.id
```

| 一边 | 来源 | 是否可信 |
|------|------|----------|
| 当前用户 | Cookie / Bearer → Session Store → `LoginUser` | 是（服务端签发与校验） |
| 资源归属 | `Model::find` / 关联加载 | 是（数据库） |
| 前端 `user_id` / `isVip` / roles | body / props / query | **否**，忽略或禁止 |

不是「把前端每个字段和数据库逐项相等比对」，而是：**用会话身份对照库记录的 `user_id`（或角色策略）**。

---

## 1. 定义 Policy

```bash
nx make policy Post
# → app/src/policies/post_policy.rs
```

示例应用中的真实实现：

```rust
use namix::prelude::*;

use crate::models::post::Post;
use crate::services::session::LoginUser;

pub struct PostPolicy;

impl Policy<LoginUser, Post> for PostPolicy {
    fn allows(&self, actor: &LoginUser, ability: Ability, resource: Option<&Post>) -> bool {
        match ability {
            Ability::Create => true, // 已登录即可；路由仍需 require_login
            Ability::ViewAny => true,
            Ability::View | Ability::Update | Ability::Delete => {
                resource.is_some_and(|post| post.user_id == actor.id)
            }
        }
    }
}
```

`Ability`：`ViewAny` / `View` / `Create` / `Update` / `Delete`。  
模型字段是 **`user_id`**（不是文档旧稿里的 `owner_id`）。

单元测试可直接构造 `LoginUser` + `Post` 测 `authorize`，无需起 HTTP（见同文件 `#[cfg(test)]`）。

---

## 2. 在控制器里授权

```rust
use namix::prelude::*;

use crate::middleware::extract::AuthUser;
use crate::models::post::Post;
use crate::policies::post_policy::PostPolicy;
use crate::services::user::UserService;
use crate::validators::post_form::PostRequest;

pub async fn update(req: Request, user: AuthUser, form: PostRequest) -> Result<Response, AppError> {
    let id = req.param("id").and_then(|s| s.parse().ok()).ok_or(AppError::NotFound)?;
    let post = Post::find(id).await.ok_or(AppError::NotFound)?;
    authorize(&*user, &PostPolicy, Ability::Update, Some(&post))?;
    UserService::new()
        .update_post(post.id, &form.title, &form.body)
        .await?;
    Ok(req.see_other_to(route::main::posts))
}

pub async fn destroy(req: Request, user: AuthUser) -> Result<Response, AppError> {
    let id = req.param("id").and_then(|s| s.parse().ok()).ok_or(AppError::NotFound)?;
    let post = Post::find(id).await.ok_or(AppError::NotFound)?;
    authorize(&*user, &PostPolicy, Ability::Delete, Some(&post))?;
    UserService::new().delete_post(post.id).await?;
    Ok(req.see_other_to(route::main::posts))
}

pub async fn create(req: Request, user: AuthUser, form: PostRequest) -> Result<Response, AppError> {
    authorize(&*user, &PostPolicy, Ability::Create, None)?;
    // user.id 来自会话；create_post 写入库，不信表单 user_id
    …
}
```

或使用 `Gate`：

```rust
Gate::for_user((*user).clone())
    .authorize(&PostPolicy, Ability::Update, Some(&post))?;
```

失败 → `AppError::Forbidden`（HTTP 403；Action 走统一错误映射）。

SSR 更新/删除表单须带 `<CsrfField />`，路由见 `web.rs`：`posts.update` / `posts.destroy`。

---

## 3. 与页面渲染的分工

| 场景 | 用什么 |
|------|--------|
| 已登录才能打开 `/posts` | 路由 `require_login` |
| 已登录不能打开 `/login` | 路由 `require_guest` |
| 首页导航：访客一套链接、VIP 多一个入口 | `AuthView::choose`，props 只含 `navLinks` / `greeting` |
| 更新属于别人的文章 | `authorize(..., Ability::Update, Some(&post_from_db))` |
| 管理后台区块是否出现在 HTML 里 | `AuthView::when_allows` 或控制器内 Policy，**不要**下发 `isAdmin: true` |

```rust
// 页面：只下发展示结果
let auth = AuthView::new(current(&req));
let admin_banner = auth.when_allows(&AdminPolicy, Ability::View, None, |_| {
    AdminBanner { text: "管理入口".into() } // 无 role 字段
});
```

列表页用 `user.load_posts()` 只取本人文章，是**查询范围**；单条写操作仍要再 `authorize`，防止伪造路径 id。

---

## 4. 反模式

| 不要 | 要 |
|------|-----|
| 信 `form.user_id` / body 里的 `is_admin` | 只用 `AuthUser` / `LoginUser` |
| 把 `isVip` 放进 Island props 让前端 `if` | 服务端拼好 `navLinks` / 区块 |
| Policy 里只看前端传来的 owner 字段 | Policy 看 **DB 模型** 上的 `user_id` |
| 仅靠隐藏按钮“保护”写接口 | 写路径必须再 `authorize` |

---

## 5. Laravel 对照

| Laravel | Namix |
|---------|--------|
| `$this->authorize('update', $post)` | `authorize(&actor, &PostPolicy, Ability::Update, Some(&post))?` |
| `PostPolicy::update` | `Policy<LoginUser, Post>::allows(..., Ability::Update, ...)` |
| `@can` / Blade | `AuthView` + SSR（不下发 can 布尔值到 JS） |
| `authorize` 中间件 | `require_login` / `require_vip` + 控制器内 `authorize` |
| 路由 model binding + Policy | 手动 `Post::find` + `authorize`（推荐显式查库） |

骨架：`nx make policy <Name>`。单元测试可直接测 `allows` / `authorize`，无需起 HTTP。
