import { Head, Link, route } from '../namix'
import type { EmailVerifyPage } from '../generated/EmailVerifyPage'
import type { PageProps } from '../types'

type Props = PageProps<EmailVerifyPage>

export default function EmailVerify({ ok, message }: Props) {
  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/40">
      <Head title="验证邮箱 · Namix" />
      <div className="mx-auto flex min-h-screen max-w-md flex-col justify-center px-6 py-14">
        <section className="rounded-2xl border border-zinc-200/80 bg-white p-8 shadow-sm">
          <h1 className="text-2xl font-semibold tracking-tight text-zinc-900">
            {ok ? '验证成功' : '验证失败'}
          </h1>
          <p className="mt-3 text-sm text-zinc-600">{message}</p>
          <Link
            href={ok ? route.me() : route.login()}
            className="mt-6 inline-block text-sm font-medium text-teal-700 hover:underline"
          >
            {ok ? '前往资料页' : '返回登录'}
          </Link>
        </section>
      </div>
    </main>
  )
}
