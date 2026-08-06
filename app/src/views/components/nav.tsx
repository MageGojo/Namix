import { Link } from '../namix'
import { route } from '../routes'

type Props = {
  username?: string | null
}

/** 登录态顶栏（SSR / SPA 共用）。 */
export function AppNav({ username }: Props) {
  return (
    <nav className="mb-8 flex flex-wrap items-center gap-4 text-sm text-zinc-600">
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
          <Link className="hover:text-zinc-900" href={route.logout()}>
            Logout ({username})
          </Link>
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
  )
}
