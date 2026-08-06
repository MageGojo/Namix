import type { HomePage } from '../generated/HomePage'
import { LoginForm } from '../generated/fields'
import { Head, Link } from '../namix'
import { route, routes } from '../routes'
import type { PageProps } from '../types'

type Props = PageProps<HomePage>

export default function Home({
  title,
  username,
  isVip = false,
  usersCount,
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
            <Link prefetch className="hover:text-zinc-900" href={route.home()}>
              Home
            </Link>
            {username ? (
              <>
                <Link prefetch className="hover:text-zinc-900" href={route.me()}>
                  Me
                </Link>
                <Link prefetch className="hover:text-zinc-900" href={route.posts()}>
                  Posts
                </Link>
                <Link prefetch className="hover:text-zinc-900" href={route.chat()}>
                  Chat
                </Link>
                <Link
                  prefetch
                  className="hover:text-zinc-900"
                  href={route.profile({ id: 1 })}
                >
                  Public
                </Link>
                <Link prefetch className="hover:text-zinc-900" href={route.demo()}>
                  Demo
                </Link>
                {isVip ? (
                  <Link prefetch className="hover:text-zinc-900" href={route.vip()}>
                    VIP
                  </Link>
                ) : null}
              </>
            ) : (
              <>
                <Link prefetch className="hover:text-zinc-900" href={route.login()}>
                  Login
                </Link>
                <Link prefetch className="hover:text-zinc-900" href={route.register()}>
                  Register
                </Link>
                <Link prefetch className="hover:text-zinc-900" href={route.demo()}>
                  Demo
                </Link>
              </>
            )}
          </nav>
        </header>

        <section className="space-y-3">
          <h1 className="text-4xl font-semibold tracking-tight">{title}</h1>
          <p className="text-lg text-zinc-600">
            {username ? (
              <>
                已登录：<span className="font-medium text-zinc-900">{username}</span>
              </>
            ) : (
              '未登录'
            )}
          </p>
          <p className="text-zinc-600">库中用户：{usersCount}</p>
          <p className="text-sm text-zinc-500">
            URL <code className="rounded bg-zinc-200 px-1.5">{url}</code>
            {' · '}
            字段 <code className="rounded bg-zinc-200 px-1.5">{LoginForm.Username}</code>
            {' · '}
            <code className="rounded bg-zinc-200 px-1.5">route.home()</code>
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
