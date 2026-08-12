# 前端交互

前端在 `app/src/views/`。公共 API 统一从 `namix.ts` 导出；页面 props / Action / 字段名由构建生成，**不要手改 `generated/`**。

```ts
import { Link, Head, useForm, usePage, router, CsrfField, csrfToken } from '../namix'
import { route } from '../routes'
import { login } from '../generated/actions/login'
import type { LoginPage } from '../generated/LoginPage'
```

---

## 设计原理

1. **Inertia 风格导航**：同站 GET 用 `Link` / `router.visit`，带 `X-Namix-Props` 只换页数据，不全量刷。
2. **契约生成**：Rust `ViewData` → TS 类型；`#[server]` → `actions/*.ts`；`FormField` → `fields.ts`。
3. **渲染模式决定挂载策略**：`.ssr()` 优先使用 Rust 正文；没有正文渲染器时会内联 props 并挂载客户端，避免空白页。需要确定交互能力时直接用 `.island()`。
4. **错误分两层**：后端给英文/稳定 key；前端 `messages` / `mapErrors` 做成产品文案。

---

## 1. 页面与 props

### Rust 侧

```rust
#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct LoginPage {
    pub error: Option<String>,
    pub redirect: String,
    pub brand_icon: String,
    pub registered_count: u64,
}

req.view("login").island().title("登录").data(LoginPage { … }).render()
```

### TS 侧

```tsx
import type { LoginPage } from '../generated/LoginPage'
import type { PageProps } from '../types'

type Props = PageProps<LoginPage>

export default function Login({
  error: initialError,
  redirect = '/me',
  brandIcon,
  registeredCount,
}: Props) {
  // brandIcon ← brand_icon（camelCase）
}
```

`PageProps<T>` 会带上框架注入的 `url` 等；业务字段来自 `ViewData`。

**不要**在 props 里塞路由表——用 `import { route }`。

---

## 2. 渲染模式（和前端能力）

| 控制器 | 前端能力 |
|--------|----------|
| `.ssr_html(html)` | 可信的 Rust 模板正文 + CSS；纯服务端 HTML，不加载客户端 React |
| `.ssr()` | SSR 优先；有 Rust 正文时输出纯 HTML，否则自动采用 Island 的内联客户端挂载，绝不返回空壳 |
| `.island()` | **Rust 壳 + 内联 props** + 客户端 mount/hydrate；可用全部交互 API（**不依赖 Node**） |
| `.spa()` | 客户端再拉 `/__namix/props`；多应用反代易串台，慎用 |

入口逻辑（概念）：只有已经得到非空 Rust 正文的页面才输出 `data-namix-mode="ssr"`；缺少正文的 `.ssr()` 会输出 `island` 模式并由 `_entry.tsx` 挂载。

因此：

- 登录 / 注册 → `.island()` + `useForm`
- Rust 模板 + 经典 form → `.ssr_html(rendered_html)` + `<form method="post">`
- React 展示页但希望 SSR 优先 → `.ssr()`；明确要 hydrate 的页面 → `.island()`

---

## 3. `useForm` + Server Action（完整用法）

### 最小闭环

```tsx
import { login } from '../generated/actions/login'
import { Head, Link, useForm } from '../namix'
import { route } from '../routes'

const LOGIN_MESSAGES: Record<string, string> = {
  'username is required': '请填写用户名',
  'password is required': '请填写密码',
  'invalid username or password': '用户名或密码不正确',
}

export default function Login({ redirect = '/me', brandIcon, registeredCount }: Props) {
  const form = useForm({
    username: '',
    password: '',
    redirect,
  })

  return (
    <form
      onSubmit={form.onSubmit(login, {
        messages: LOGIN_MESSAGES,
        mapErrors: (errors) => {
          // 演示：密码错时也高亮用户名
          if (errors.password === '用户名或密码不正确' && !errors.username) {
            return { ...errors, username: '请检查用户名' }
          }
          return errors
        },
        onError: (errors) => console.debug(errors),
        // followRedirect: true（默认）— 跟随 ActionOk.redirect
      })}
      noValidate
    >
      <input
        name="username"
        value={form.data.username}
        onChange={(e) => {
          form.setData('username', e.target.value)
          form.clearErrors('username', '_')
        }}
        aria-invalid={!!form.errors.username}
      />
      {form.errors.username ? <p>{form.errors.username}</p> : null}

      <input
        name="password"
        type="password"
        value={form.data.password}
        onChange={(e) => {
          form.setData('password', e.target.value)
          form.clearErrors('password', '_')
        }}
      />
      {form.errors.password ? <p>{form.errors.password}</p> : null}

      <button type="submit" disabled={form.processing}>
        {form.processing ? '登录中…' : '登录'}
      </button>
    </form>
  )
}
```

### `useForm` 能力一览

| API | 作用 |
|-----|------|
| `data` / `setData` | 表单状态 |
| `errors` / `error` / `clearErrors` | 字段袋；`_` 为总错误 |
| `processing` | 提交中 |
| `onSubmit(action, opts)` | 绑生成的 `login` / `register` |
| `submit(action, opts)` | 命令式提交 |
| `reset` / `get` | 重置或取字段 |

`action` 来自 `generated/actions/*`，内部走 `callRust` → `POST /api/a`（可选 WASM 密封）。

### 与后端错误的对应

```text
ActionError::field("password", "invalid…")
  → errors.password
  → messages 映射成中文
  → mapErrors 可再改写

ActionError::message("…")
  → errors._
```

---

## 4. 经典 HTML 表单（SSR）与 CSRF

无 hydrate 时用浏览器原生提交。Boot 默认开启 **Origin + double-submit CSRF**：经典 `<form method="post">` **必须**带隐藏字段 `_csrf`（可读 Cookie `namix_csrf`）。

| 入口 | CSRF 怎么带 |
|------|-------------|
| 经典 HTML 表单 | `<CsrfField />`（或手写 `<input type="hidden" name="_csrf" value={…} />`） |
| `useForm` / `generated/actions/*` | Action 客户端自动带同一 token，业务页不必手写 |
| 纯 Bearer API | 无 Cookie 会话时自动豁免（见安全文档） |

```tsx
import { CsrfField } from '../namix'
import { PostForm } from '../generated/fields'
import { route } from '../routes'

<form method="post" action={route.posts.submit()}>
  <CsrfField />
  <input name={PostForm.Title} />
  <textarea name={PostForm.Body} rows={4} />
  <button type="submit">发布</button>
</form>

{/* 登出、更新、删除同理 */}
<form method="post" action={route.logout()}>
  <CsrfField />
  <button type="submit">退出</button>
</form>
```

后端（示例 `posts` 已挂 `require_login` + `PostPolicy`）：

```rust
pub async fn create(req: Request, user: AuthUser, form: PostRequest) -> Result<Response, AppError> {
    authorize(&*user, &PostPolicy, Ability::Create, None)?;
    match UserService::new().create_post(user.id, &form.title, &form.body).await {
        Ok(_) => Ok(req.see_other_to(route::main::posts)),
        Err(error) => Ok(req.redirect_error_to(route::main::posts, error.message())),
    }
}
```

页面用 `error` props（来自 flash）展示失败信息。`csrfToken()` 可读当前 Cookie，供非 React 脚本使用。

---

## 5. `Link` / `router` / 进度条

```tsx
import { Link, router } from '../namix'
import { route } from '../routes'

<Link href={route.home()} prefetch>
  首页
</Link>

<button type="button" onClick={() => void router.reload()}>
  刷新 props
</button>

void router.visit(route.posts())
void router.prefetch(route.me())
```

| 能力 | 说明 |
|------|------|
| `Link` | 同站 GET 软导航；Ctrl/⌘ 点击仍走浏览器默认 |
| `prefetch` | 悬停约 75ms 预取 props |
| `router.visit` | 编程式跳转 + 顶栏进度条 |
| `router.reload` | 重新拉当前页 props |
| `progress` | NProgress 风格顶栏（`configureProgress` 可改色） |

软导航失败（非 JSON）会回退为 `location.href` 整页加载。

注意：目标页若 props 缺少数组字段，前端 `items.length` 可能崩——框架会保留空数组；页面侧也可 `items = []` 做默认。

---

## 6. `Head` / `usePage`

```tsx
import { Head, usePage, router } from '../namix'
import type { MePage } from '../generated/MePage'

export default function Me(props: PageProps<MePage>) {
  const page = usePage<MePage>()

  return (
    <main>
      <Head title={`${props.title} · Namix`} />
      <p>
        usePage · {page.component} ·{' '}
        <button type="button" onClick={() => void router.reload()}>
          reload
        </button>
      </p>
      {/* … */}
    </main>
  )
}
```

`usePage` 必须在 `PageProvider` 下（`_entry` 已包好）。软导航后 `page.component` / `page.props` 会更新。

纯 SSR 页没有客户端 `Head` 生效路径时，用控制器 `.title("…")` 设置文档标题。

---

## 7. 资源与静态文件

- 图片等：**Vite `import logo from '../assets/x.svg?url'`**，不要写死 `/build/assets/xxx.svg`（哈希会变；小 SVG 可能被内联成 data URL）。
- 生产静态默认挂在 `/build/*`，由 Namix 按 Vite manifest 提供。
- **子路径挂载**（应用对外入口是 `/lr` 等、反代只转发该前缀）：运行时与构建都设同一前缀，否则 JS 404 白屏：
  - 运行：`NAMIX_ASSET_PREFIX=/lr`（或 `NAMIX_ASSET_BASE=/lr/build`）
  - 构建：同上环境变量（脚手架 `vite.config` 已读）；磁盘仍是 `public/build/`
  - 效果：标签与路由变为 `/lr/build/…`，同时保留根 `/build/*` 供直连端口
- `publicDir: false`：不要把「唯一真相」只丢在 `public/build` 却不走打包。

---

## 8. 生成物一览（勿手改）

| 路径 | 来源 |
|------|------|
| `generated/LoginPage.ts` 等 | `ViewData` / 页面 DTO |
| `generated/actions/login.ts` | `#[server]` |
| `generated/callRust.ts` | namix-build + seal WASM |
| `generated/fields.ts` | `FormField` |
| `generated/registry.ts` | `pages/*.tsx` 扫描 |
| `routes.ts` | 命名路由 catalog |

改 Rust / 页面后重新 `nx dev` 或 build，生成物会更新。

---

## 9. 新建交互页清单

**Island + Action（登录类）**

1. `validators/xxx_form.rs`
2. 控制器 GET：`.island().data(ViewData).render()`
3. `#[server]` + `from_values` + `ActionOk` / `ActionError`
4. `web.rs` 只注册 GET
5. `pages/xxx.tsx`：`useForm` + `generated/actions`
6. 用 `messages` 做中文

**SSR + 经典表单（资料类）**

1. 验证器 `FormRedirect::Named("…")`
2. GET `.ssr()` + POST 路由
3. 页面 `<form method="post" action={route.xxx.submit()}>`
4. 成功/失败 `redirect_ok_to` / `redirect_error_to`

---

## 10. 端到端对照（登录）

```text
login.tsx
  useForm({ username, password, redirect })
  onSubmit(login)  ──► generated/actions/login.ts
                         callRust(token, input)
                           POST /api/a  (seal 可选)
                             auth::login_action
                               LoginRequest::from_values
                               UserService::authenticate
                               rotate_pair → Cookie opaque + JWT
                               ActionOk {
                                 redirect,
                                 access_token?, token_type?, expires_in?
                               } + Set-Cookie namix_session (Max-Age = lifetime_secs)
                         ◄── errors 字段袋 | redirect (+ tokens 给 API/mobile)
  router.visit('/me')  （软导航；浏览器跟 Cookie，不必手存 JWT）
    GET /me + X-Namix-Props
      me::show → props JSON
        渲染 Me 组件
```

---

## 易错点

| 问题 | 正确做法 |
|------|----------|
| SSR 页白屏却在用 `useForm` | 控制器改 `.island()` |
| 手写 Action URL | 只用 `generated/actions` |
| props 里带 `routes` | 用 `import { route }` |
| 硬编码 `/build/assets/namix.svg` | `import …?url` |
| 空列表页软导航白屏 | 依赖框架保留 `[]`，或 `items = []` |
| 在 Node SSR 里调 `callRust` | Action 仅浏览器；SSR 用经典 POST 或只渲染 |
