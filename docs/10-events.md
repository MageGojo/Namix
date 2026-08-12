# 事件与监听器

领域事件把「主流程已成功」与「副作用」拆开：控制器 `dispatch`，各功能在启动时 `listen`。目录由 `[features] events` / `listeners` 控制（见 [FEATURES.md](./FEATURES.md)）。

---

## 1. 定义事件

```rust
// app/src/events/user_registered.rs
#[derive(Clone, Debug)]
pub struct UserRegistered {
    pub user_id: u64,
    pub username: String,
}
```

示例另有 `UserLoggedIn { user_id, username, ip }`。

---

## 2. 挂监听器

```rust
// app/src/listeners/register.rs
use namix::prelude::*;
use crate::events::user_registered::UserRegistered;

pub fn all() {
    listen(|e: &UserRegistered| {
        let to = format!("{}@users.namix.local", e.username);
        match Mail::send(
            MailMessage::new(to, "欢迎加入 Namix")
                .text(format!("你好 {}，注册成功。", e.username)),
        ) {
            Ok(()) => Reply::ok("welcome mail"),
            Err(err) => Reply::err(format!("welcome mail failed: {err}")),
        }
    });

    listen(|e: &UserRegistered| {
        namix::log::info!("audit: user registered #{}", e.user_id);
        Reply::ok("audit")
    });
}
```

在 `main.rs`（或 bin）启动时调用：

```rust
listeners::register::all();
listeners::login::all();
```

---

## 3. 派发

```rust
// 注册成功、落库之后
let outcome = dispatch(UserRegistered {
    user_id: user.id,
    username: user.username.clone(),
});
// 可按 outcome 打日志；部分监听失败不应悄悄回滚已提交的用户行
```

原则：

1. **先写完事务/主状态，再 dispatch**。
2. 监听器失败用 `Reply::err` + 日志；是否重试交给队列 Job（见 [平台 · Queue](./08-platform.md)）。
3. 不要在监听器里再改「决定事件是否成立」的核心不变量（除非有明确补偿设计）。

---

## 4. 与示例对照

| 事件 | 派发点 | 监听 |
|------|--------|------|
| `UserRegistered` | `auth` 注册 Action | 欢迎邮件 / 审计 / profile seed |
| `UserLoggedIn` | `auth` 登录 Action | 审计 / 新环境提醒 |

目录带 `.namix-feature`；关掉 feature 后下次构建会移除带标记的目录。
