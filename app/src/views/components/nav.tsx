import { CsrfField, Link, route, toggleTheme } from '../namix'

type Props = {
  username?: string | null
}

/** 登录态顶栏（SSR / SPA 共用）。 */
export function AppNav({ username }: Props) {
  return (
    <nav className="mb-8 flex flex-wrap items-center gap-4 text-sm text-zinc-600 dark:text-zinc-300">
      <Link
        prefetch
        className="font-medium text-teal-700 hover:text-teal-900"
        href={route.home()}
      >
        Namix
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
          <Link prefetch className="hover:text-zinc-900" href={route.mailbox()}>
            Mail
          </Link>
          <Link prefetch className="hover:text-zinc-900" href={route.demo()}>
            Demo
          </Link>
          <form method="post" action={route.logout()}>
            <CsrfField />
            <button type="submit" className="hover:text-zinc-900">
              Logout ({username})
            </button>
          </form>
        </>
      ) : (
        <>
          <Link prefetch className="hover:text-zinc-900" href={route.login()}>
            Login
          </Link>
          <Link prefetch className="hover:text-zinc-900" href={route.register()}>
            Register
          </Link>
          <Link prefetch className="hover:text-zinc-900 dark:hover:text-zinc-100" href={route.demo()}>
            Demo
          </Link>
        </>
      )}
      <button
        type="button"
        className="ml-auto hover:text-zinc-900 dark:hover:text-zinc-100"
        onClick={() => toggleTheme()}
      >
        主题
      </button>
    </nav>
  )
}
