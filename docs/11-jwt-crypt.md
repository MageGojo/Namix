# JWT 与 Crypt

浏览器会话以 **opaque 签名 Cookie** 为主；API 可选用 **HS256 JWT Bearer**。敏感闪存与 Cookie 值可用 **`namix::Crypt`**（AES-256-GCM）密封。密钥均来自 `security.session_secret` / `NAMIX_SESSION_SECRET`。

---

## 1. 双通道会话

```text
登录成功
  ├─ Cookie：namix_session = HMAC(opaque sid)   Max-Age = lifetime_secs
  └─ JSON：access_token = JWT(sub, sid, …)      exp ≈ jwt_lifetime_secs
           共用 SessionStore 里同一条 sid
登出 / logout_all
  └─ revoke(sid) 或 revoke_all_for_user → Cookie 与 Bearer 同时失效
```

```toml
[session]
driver = "memory"          # 开发；生产滚动用 file / redis
lifetime_secs = 604800
jwt_lifetime_secs = 3600
```

业务侧：`SessionService::rotate_pair` / `issue_pair` 一次给出 `cookie_token` + `access_token`。中间件 `hydrate` 接受 Cookie 或 `Authorization: Bearer`（opaque 或 JWT）。

---

## 2. JWT API

```rust
use namix::{Jwt, JwtClaims};

let token = Jwt::encode(&claims, secret)?;
let claims = Jwt::decode(&token, secret)?;   // 验签 + exp
let claims = Jwt::decode_ignore_exp(&token, secret)?;

// claims.sub / .sid / .username / .is_vip / .iat / .exp
// JwtClaims::from_session(session_id, &session, ttl)
```

| 错误 | 映射 |
|------|------|
| 畸形 / 签名坏 / 过期 | `AppError::Unauthenticated` |
| 其它编解码 | `AppError::internal` |

`Jwt::looks_like` 可粗判字符串是否像 JWT。claims 里的 **`sid` 必须能在 Store 查到**，否则登出无法撤销。

前端浏览器跟 Cookie + `redirect` 即可；移动端/API 存 `access_token`，请求头带 `Authorization: Bearer …`。

---

## 3. Crypt API

```rust
use namix::Crypt;

// Boot 已用 session secret 调用 Crypt::install
let sealed = Crypt::encrypt_str("flash-or-blob")?;
let plain = Crypt::decrypt_str(&sealed)?;

let cookie_val = Crypt::seal_cookie_value("raw")?;
let raw = Crypt::open_cookie_value(&cookie_val)?; // 未 install 时明文透传
```

- 密文前缀 `nx1:`（AES-256-GCM + URL-safe base64）。
- **只在服务端解密**；不要把「角色 / 是否管理员」封进密文交给浏览器再解开当授权依据——授权仍走会话 + Policy。
- Flash 由框架经 Crypt 密封；业务自定义 Cookie 可用 `seal_cookie_value`。

---

## 4. Action 密封（相关但不同）

`[features] action_seal` / `NAMIX_ACTION_SEAL` 控制 `#[server]` 传输层字段加密（`seal = ["password"]`）。那是 Action 通道密钥，与 Crypt 的 session 派生键是两套用途；生产均应开启。

---

## 5. 对照

| Laravel | Namix |
|---------|--------|
| Cookie session | opaque `namix_session` + `SessionStore` |
| Sanctum / Passport token | 可选 HS256 JWT（同 sid） |
| `Crypt::encrypt` | `namix::Crypt` |
| `Auth::logoutOtherDevices` | `revoke_all_for_user` / `logout_all` Action |
