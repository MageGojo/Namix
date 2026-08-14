import type { RegisterPage } from '../generated/RegisterPage'
import { register } from '../generated/actions/register'
import { Head, Link, useForm, route } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<RegisterPage>

function fieldClass(invalid: boolean) {
  return invalid
    ? 'w-full rounded-lg border border-red-400 px-3 py-2 text-zinc-900 outline-none ring-red-500/30 focus:border-red-500 focus:ring-2'
    : 'w-full rounded-lg border border-zinc-300 px-3 py-2 text-zinc-900 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2'
}

export default function Register({ error: initialError }: Props) {
  const form = useForm({
    username: '',
    email: '',
    password: '',
    password_confirmation: '',
  })

  const formError = form.errors._ || initialError

  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/40">
      <Head title="注册 · Namix" />
      <div className="mx-auto flex min-h-screen max-w-md flex-col justify-center px-6 py-14">
        <Link
          href={route.home()}
          prefetch
          className="mb-8 text-sm font-medium tracking-wide text-teal-700 hover:text-teal-900"
        >
          ← Namix
        </Link>

        <section className="rounded-2xl border border-zinc-200/80 bg-white p-8 shadow-sm">
          <h1 className="text-2xl font-semibold tracking-tight text-zinc-900">注册</h1>
          <p className="mt-2 text-sm text-zinc-500">
            密码 ≥8，含大小写、数字、特殊字符。例：Secret1!
          </p>

          {formError ? (
            <p className="mt-4 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-700">{formError}</p>
          ) : null}

          <form
            onSubmit={form.onSubmit(register, {
              mapErrors: (errors) => {
                if (errors.password === 'password.confirmed' && !errors.password_confirmation) {
                  return { ...errors, password_confirmation: 'password.confirmed' }
                }
                return errors
              },
            })}
            className="mt-6 space-y-4"
            noValidate
          >
            <label className="block space-y-1.5">
              <span className="text-sm font-medium text-zinc-700">用户名</span>
              <input
                name="username"
                autoComplete="username"
                value={form.data.username}
                onChange={(e) => {
                  form.setData('username', e.target.value)
                  form.clearErrors('username', '_')
                }}
                aria-invalid={!!form.errors.username}
                className={fieldClass(!!form.errors.username)}
              />
              {form.errors.username ? (
                <p className="text-sm text-red-600">{form.errors.username}</p>
              ) : null}
            </label>
            <label className="block space-y-1.5">
              <span className="text-sm font-medium text-zinc-700">邮箱</span>
              <input
                name="email"
                type="email"
                autoComplete="email"
                value={form.data.email}
                onChange={(e) => {
                  form.setData('email', e.target.value)
                  form.clearErrors('email', '_')
                }}
                aria-invalid={!!form.errors.email}
                className={fieldClass(!!form.errors.email)}
              />
              {form.errors.email ? (
                <p className="text-sm text-red-600">{form.errors.email}</p>
              ) : null}
            </label>
            <label className="block space-y-1.5">
              <span className="text-sm font-medium text-zinc-700">密码</span>
              <input
                name="password"
                type="password"
                autoComplete="new-password"
                value={form.data.password}
                onChange={(e) => {
                  form.setData('password', e.target.value)
                  form.clearErrors('password', 'password_confirmation', '_')
                }}
                aria-invalid={!!form.errors.password}
                className={fieldClass(!!form.errors.password)}
              />
              {form.errors.password ? (
                <p className="text-sm text-red-600">{form.errors.password}</p>
              ) : null}
            </label>
            <label className="block space-y-1.5">
              <span className="text-sm font-medium text-zinc-700">确认密码</span>
              <input
                name="password_confirmation"
                type="password"
                autoComplete="new-password"
                value={form.data.password_confirmation}
                onChange={(e) => {
                  form.setData('password_confirmation', e.target.value)
                  form.clearErrors('password_confirmation', 'password', '_')
                }}
                aria-invalid={!!form.errors.password_confirmation}
                className={fieldClass(!!form.errors.password_confirmation)}
              />
              {form.errors.password_confirmation ? (
                <p className="text-sm text-red-600">{form.errors.password_confirmation}</p>
              ) : null}
            </label>
            <button
              type="submit"
              disabled={form.processing}
              className="w-full rounded-lg bg-teal-700 px-4 py-2.5 text-sm font-medium text-white hover:bg-teal-800 disabled:opacity-60"
            >
              {form.processing ? '注册中…' : '注册'}
            </button>
          </form>

          <p className="mt-6 text-center text-sm text-zinc-500">
            已有账号？{' '}
            <Link
              href={route.login()}
              prefetch
              className="font-medium text-teal-700 hover:underline"
            >
              登录
            </Link>
            {' · '}
            <Link href={route.oauth.redirect({ provider: 'dev' })} className="font-medium text-teal-700 hover:underline">
              Dev 登录
            </Link>
          </p>
        </section>
      </div>
    </main>
  )
}
