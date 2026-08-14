import brandFallback from '../assets/namix.svg?url'
import type { LoginPage } from '../generated/LoginPage'
import { login } from '../generated/actions/login'
import { Head, Link, useForm, route } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<LoginPage>

export default function Login({
  error: initialError,
  redirect = '/me',
  brandIcon,
  registeredCount,
}: Props) {
  const form = useForm({
    username: '',
    password: '',
    redirect,
  })

  const formError = form.errors._ || initialError

  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/40">
      <Head title="登录 · Namix" />
      <div className="mx-auto flex min-h-screen max-w-md flex-col justify-center px-6 py-14">
        <Link
          href={route.home()}
          prefetch
          className="mb-8 text-sm font-medium tracking-wide text-teal-700 hover:text-teal-900"
        >
          ← Namix
        </Link>

        <section className="rounded-2xl border border-zinc-200/80 bg-white p-8 shadow-sm">
          <div className="flex items-center gap-3">
            <img
              src={brandIcon || brandFallback}
              alt="Namix"
              className="h-8 w-8"
              width={32}
              height={32}
            />
            <h1 className="text-2xl font-semibold tracking-tight text-zinc-900">登录</h1>
          </div>
          <p className="mt-2 text-sm text-zinc-500">
            种子账号：alice / Secret1!
            {typeof registeredCount === 'number' ? ` · 已注册 ${registeredCount} 人` : null}
          </p>

          {formError ? (
            <p className="mt-4 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-700">{formError}</p>
          ) : null}

          <form
            onSubmit={form.onSubmit(login, {
              mapErrors: (errors) => {
                if (errors.password === 'auth.failed' && !errors.username) {
                  return { ...errors, username: 'auth.check_username' }
                }
                return errors
              },
              onError: (errors) => {
                console.debug('[login] action errors', errors)
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
                className={
                  form.errors.username
                    ? 'w-full rounded-lg border border-red-400 px-3 py-2 text-zinc-900 outline-none ring-red-500/30 focus:border-red-500 focus:ring-2'
                    : 'w-full rounded-lg border border-zinc-300 px-3 py-2 text-zinc-900 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2'
                }
              />
              {form.errors.username ? (
                <p className="text-sm text-red-600">{form.errors.username}</p>
              ) : null}
            </label>
            <label className="block space-y-1.5">
              <span className="text-sm font-medium text-zinc-700">密码</span>
              <input
                name="password"
                type="password"
                autoComplete="current-password"
                value={form.data.password}
                onChange={(e) => {
                  form.setData('password', e.target.value)
                  form.clearErrors('password', '_')
                }}
                aria-invalid={!!form.errors.password}
                className={
                  form.errors.password
                    ? 'w-full rounded-lg border border-red-400 px-3 py-2 text-zinc-900 outline-none ring-red-500/30 focus:border-red-500 focus:ring-2'
                    : 'w-full rounded-lg border border-zinc-300 px-3 py-2 text-zinc-900 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2'
                }
              />
              {form.errors.password ? (
                <p className="text-sm text-red-600">{form.errors.password}</p>
              ) : null}
            </label>
            <button
              type="submit"
              disabled={form.processing}
              className="w-full rounded-lg bg-teal-700 px-4 py-2.5 text-sm font-medium text-white hover:bg-teal-800 disabled:opacity-60"
            >
              {form.processing ? '登录中…' : '登录'}
            </button>
          </form>

          <p className="mt-6 text-center text-sm text-zinc-500">
            还没有账号？{' '}
            <Link
              href={route.register()}
              prefetch
              className="font-medium text-teal-700 hover:underline"
            >
              注册
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
