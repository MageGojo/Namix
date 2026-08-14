# 验证器

表单校验放在 `app/src/validators/`。核心抽象：

- `FormField`：字段枚举（防拼写错误）
- `FormRequest`：校验通过后的结构化输入
- `Rule` / `custom`：规则链
- 两种失败出口：**闪存跳转**（经典 POST）或 **ActionError 字段袋**（`#[server]`）

---

## 设计原理

1. **进控制器即合法**：`FormRequest` 通过后，业务只拿强类型字段，不再到处 `if username.is_empty()`。
2. **字段名单一来源**：`#[field = "username"]` → Rust 枚举 + 生成 `views/generated/fields.ts`。
3. **失败策略与入口绑定**
   - 提取器 `form: ProfileRequest` → 按 `redirect_to()` 闪存跳转
   - `from_values(&req)?` 在 `#[server]` 里 → 变成 JSON `errors`
4. **`from_values` 内不要自己 redirect**：只返回 `Result<Self, ValidationError>`。

---

## 1. 完整示例：注册表单

```rust
//! app/src/validators/register_form.rs

use crate::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum RegisterForm {
    #[field = "username"]
    Username,
    #[field = "password"]
    Password,
    #[field = "password_confirmation"]
    PasswordConfirmation,
}

#[derive(Clone, Debug)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
}

impl FormRequest for RegisterRequest {
    fn redirect_to() -> FormRedirect {
        FormRedirect::named(AppRoute::Register)  // 经典 POST 失败时跳回注册页
    }

    fn from_values(req: &Request) -> Result<Self, ValidationError> {
        let v = req
            .validator()
            .rules(
                RegisterForm::Username,
                &[
                    Rule::Required,
                    Rule::Between(3, 16),
                    Rule::Regex(r"^[a-zA-Z0-9_]+$"),
                ],
            )
            .rules(
                RegisterForm::Password,
                &[Rule::Required, Rule::Min(8), Rule::Confirmed],
            )
            .custom(RegisterForm::Password, |password, _| {
                let has_upper = password.chars().any(|c| c.is_ascii_uppercase());
                let has_lower = password.chars().any(|c| c.is_ascii_lowercase());
                let has_digit = password.chars().any(|c| c.is_ascii_digit());
                let has_special = password.chars().any(|c| !c.is_ascii_alphanumeric());
                if has_upper && has_lower && has_digit && has_special {
                    Ok(())
                } else {
                    Err("password must include upper, lower, digit and special char".into())
                }
            })
            .validate()?;

        Ok(Self {
            username: v.get(RegisterForm::Username).to_string(),
            password: v.get(RegisterForm::Password).to_string(),
        })
    }
}
```

`Rule::Confirmed` 会自动看 `password_confirmation` 字段。

---

## 2. 两种使用方式

### A. `#[server]`：手动 `from_values`

```rust
#[server(name = "register", seal = ["password", "password_confirmation"])]
pub async fn register_action(req: Request) -> Result<ActionOk<AuthOk>, AppError> {
    let form = RegisterRequest::from_values(&req)?;
    // form.username / form.password 已合法
    …
}
```

校验失败 → `ValidationError` → `AppError` / `ActionError` → 字段值为**稳定码**（`username.required`）。前端 `useForm` 用 `t(code)` 读 `lang/*.json`。

业务级错误继续用字段袋（或 `AppError::validation`）：

```rust
Err(ActionError::field("username", "username.taken"))
// 或 Err(AppError::validation("username", "username.taken"))
```

### B. 经典 POST：提取器注入

```rust
pub async fn create(req: Request, user: AuthUser, form: PostRequest) -> Response {
    // 能进到这里说明校验已通过
    UserService::new()
        .create_post(user.id, &form.title, &form.body)
        .await
        …
}
```

失败时框架根据 `FormRedirect`：

```rust
fn redirect_to() -> FormRedirect {
    FormRedirect::named(AppRoute::Posts)  // 或 FormRedirect::Back / FormRedirect::Named("posts")
}
```

自动 `redirect` + flash error，**不会**返回 JSON。

---

## 3. 常用 Rule

| Rule | 含义 |
|------|------|
| `Required` | 非空 |
| `Min(n)` / `Max(n)` / `Between(a, b)` | 长度或数值范围 |
| `Email` | 邮箱格式 |
| `Numeric` / `Integer` / `Digits(n)` | 数字 |
| `AlphaNum` | 字母数字 |
| `Url` | URL |
| `LocalPath` | 站内绝对路径；拒绝外部 URL、`//host` 和反斜杠变体 |
| `Boolean` / `Accepted` / `Declined` | 布尔类 |
| `Confirmed` | 需有 `{field}_confirmation` 且相等 |
| `Same(other_field)` | 与另一字段相同 |
| `In(&[…])` / `NotIn` | 枚举白/黑名单 |
| `Regex(r"…")` | 正则 |
| `StartsWith` / `EndsWith` / `Eq` / `NotEq` | 字符串比较 |

自定义：

```rust
.custom(MyForm::Title, |value, all| {
    if value.contains("spam") {
        Err("title.spam".into())
    } else {
        Ok(())
    }
})
```

`all` 可取其它字段做交叉校验。同一字段多条规则：**先失败先停**。

---

## 4. 读取校验结果

```rust
let v = req.validator().rules(…).validate()?;

v.get(MyForm::Title)           // &str（字段必须存在）
v.get_or(MyForm::Title, "")    // 带默认
v.raw("title")                 // 按字符串 key
v.local_path_or("redirect", "/me") // 站内跳转，非法值回退默认路径
```

---

## 5. 登录里的业务防护示例

开放重定向：只允许站内相对路径。

```rust
// login_form.rs 思路
let redirect = v.local_path_or("redirect", "/me").to_string();
```

把 `redirect` 放进 `LoginRequest`，Action 成功后再跳转。

---

## 6. unique / exists / 文件字段

```rust
Rule::unique("users", "username"),
Rule::unique("profiles", "email"),
Rule::unique_ignore_col("profiles", "email", "user_id", current_user_id.to_string()),
Rule::exists("users", "username"),
Rule::Image,
Rule::Mimes(&["png", "jpg", "jpeg", "webp"]),
Rule::MaxBytes(2_000_000),
```

- unique / exists 走 `PresenceVerifier`（SQLite 在 Boot 连库后自动安装）。空值跳过。
- 文件字段走经典 `multipart/form-data` + `<CsrfField />`（`#[server]` / `useForm` 仍是 JSON，不带文件）。
- `Validated::file_field(ProfileForm::Avatar)` 取出 `UploadedFile`。
- 失败返回稳定码（`username.taken`、`email.required`），不是英文句子。改 `Min(3)` 只动 `lang/*.json` 里的 `username.min` / `validation.min`，前端不必跟句子。
- `trans_error` / `t()` 查找：精确键 → `validation.{rule}`（`:attribute` 可用 `attributes.username` 换成「用户名」）。
- `useForm.messages` 按码覆盖特例。

---

## 7. 与前端字段对齐

Rust：

```rust
#[field = "username"]
Username,
```

生成 TS（勿手改）：

```ts
export const RegisterForm = {
  Username: "username",
  Password: "password",
  // …
} as const
```

SSR 表单：

```tsx
<input name={PostForm.Title} />
```

`useForm` 的 `data` key 必须与 `#[field]` 一致，否则错误挂不到输入框上。

---

## 8. 新建验证器清单

```bash
nx make validator Checkout
# → app/src/validators/checkout_form.rs
```

然后：

1. 补全 `FormField` 枚举与 `#[field]`
2. `impl FormRequest`：`redirect_to` + `from_values`
3. 在控制器：`from_values` 或提取器参数
4. 需要时前端用生成的 `XxxForm` 常量

---

## 易错点

| 问题 | 正确做法 |
|------|----------|
| `#[server]` 里写 `form: RegisterRequest` 参数 | 改用 `from_values(&req)?` |
| 错误码直接给用户 | 默认 `t("username.taken")` 走 `lang/{locale}.json`；页面可用 `messages: { 'username.taken': '换一个' }` 覆盖 |
| `Confirmed` 但前端字段名写错 | 必须是 `{name}_confirmation` |
| 在 `from_values` 里 `return req.redirect…` | 只返回 `Err(ValidationError)` |
| 密码规则只写在前端 | 后端 `Rule` / `custom` 必须有，前端校验只是体验 |
| `.custom` 返回英文句子 | 返回码，如 `password.complexity`，并写入 `lang/*.json` |
| 经典 POST 缺 CSRF | 表单加 `<CsrfField />`；校验本身不负责 CSRF（Boot 中间件） |
