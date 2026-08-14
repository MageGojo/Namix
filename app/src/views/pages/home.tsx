import type { HomePage } from '../generated/HomePage'
import { LoginForm } from '../generated/fields'
import { Head, Link, routes } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<HomePage>

/**
 * 首页只渲染服务端已定稿的展示数据。
 * 不含 userId / isVip / roles —— 导航与问候由控制器按身份拼好。
 */
export default function Home({
  title,
  greeting,
  usersCount,
  navLinks = [],
  url = '/',
}: Props) {
  const routeNames = Object.keys(routes)
    .filter((name) => !name.startsWith('__'))
    .sort()

  return (
    <main className="min-h-screen">
      <Head title={title ? `${title} · Namix` : 'Namix'} />
      <div className="mx-auto flex max-w-3xl flex-col gap-8 px-6 py-14">
        <header className="flex flex-wrap items-center justify-between gap-4">
          <p className="text-sm font-medium tracking-wide text-teal-700">Namix</p>
          <nav className="flex flex-wrap gap-4 text-sm text-zinc-600">
            {navLinks.map((item) => (
              <Link
                key={`${item.label}:${item.href}`}
                prefetch
                className="hover:text-zinc-900"
                href={item.href}
              >
                {item.label}
              </Link>
            ))}
          </nav>
        </header>

        <section className="space-y-3">
          <h1 className="text-4xl font-semibold tracking-tight">{title}</h1>
          <p className="text-lg text-zinc-600">{greeting}</p>
          <p className="text-zinc-600">库中用户：{usersCount}</p>
          <p className="text-sm text-zinc-500">
            URL <code className="rounded bg-zinc-200 px-1.5">{url}</code>
            {' · '}
            字段 <code className="rounded bg-zinc-200 px-1.5">{LoginForm.Username}</code>
          </p>
        </section>

        <section className="space-y-3">
          <h2 className="text-lg font-medium">Named routes</h2>
          <p className="text-sm text-zinc-500">
            来自生成文件 <code className="rounded bg-zinc-200 px-1">routes.ts</code>
            ，不在页面 props 里。
          </p>
          <ul className="divide-y divide-zinc-200 rounded-lg border border-zinc-200 bg-white">
            {routeNames.map((name) => (
              <li
                key={name}
                className="flex flex-wrap items-baseline justify-between gap-2 px-4 py-2.5 text-sm"
              >
                <code className="text-teal-800">{name}</code>
                <span className="font-mono text-zinc-500">
                  {routes[name as keyof typeof routes].methods.join('|')}{' '}
                  {routes[name as keyof typeof routes].uri}
                </span>
              </li>
            ))}
          </ul>
        </section>
      </div>
    </main>
  )
}
