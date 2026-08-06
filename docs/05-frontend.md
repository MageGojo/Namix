# 前端交互

前端在 `app/src/views/`。公共 API 统一从 `namix.ts` 导出；页面 props / Action / 字段名由构建生成，**不要手改 `generated/`**。

```ts
import { Link, Head, useForm, usePage, router } from '../namix'
import { route } from '../routes'
import { login } from '../generated/actions/login'
import type { LoginPage } from '../generated/LoginPage'
```

---

## 设计原理

1. **Inertia 风格导航**：同站 GET 用 `Link` / `router.visit`，带 `X-Namix-Props` 只换页数据，不全量刷。
2. **契约生成**：Rust `ViewData` → TS 类型；`#[server]` → `actions/*.ts`；`FormField` → `fields.ts`。
3. **渲染模式决定能不能交互**：纯 `.ssr()` 没有客户端 JS；登录要用 `.island()`。
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
| `.ssr()` | 只有 HTML+CSS；无 `useForm` / hydrate |
| `.island()` | SSR HTML + 整页 hydrate；可用全部交互 API |
| `.spa()` | 客户端挂载后再渲染 |

入口逻辑（概念）：`data-namix-mode="ssr"` 时 `_entry.tsx` 直接返回，不挂 React。

因此：

- 登录 / 注册 → `.island()` + `useForm`
- 资料 / 文章列表（经典 form）→ `.ssr()` + `<form method="post">`
- 演示分页纯渲染 → `.ssr()`；要 hydrate 的分页演示 → `.island()`

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

## 4. 经典 HTML 表单（SSR）

无 hydrate 时用浏览器原生提交：

```tsx
import { PostForm } from '../generated/fields'
import { route } from '../routes'

<form method="post" action={route.posts.submit()}>
  <input name={PostForm.Title} />
  <textarea name={PostForm.Body} rows={4} />
  <button type="submit">发布</button>
</form>
```

后端：

```rust
Route::post("/posts", posts::create)
    .middleware(require_login)
    .name("posts.submit")
    .register()

pub async fn create(req: Request, user: AuthUser, form: PostRequest) -> Response {
    match UserService::new().create_post(user.id, &form.title, &form.body).await {
        Ok(_) => req.see_other_to(route::main::posts),
        Err(msg) => req.redirect_error_to(route::main::posts, msg),
    }
}
```

页面用 `error` props（来自 flash）展示失败信息。

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
- 生产静态挂在 `/build/*`，由 Namix 按 Vite manifest 提供。
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
                               ActionOk { redirect } + Set-Cookie
                         ◄── errors 字段袋 | redirect
  router.visit('/me')  （软导航）
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