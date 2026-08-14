import type { ErrorsPage } from '../generated/ErrorsPage'
import { Head, Link, route } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<ErrorsPage>

export default function Errors({ status, title, message }: Props) {
  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/30 dark:from-zinc-950 dark:to-zinc-900">
      <Head title={`${title} · Namix`} />
      <div className="mx-auto flex min-h-screen max-w-lg flex-col justify-center px-6 py-14">
        <p className="text-sm font-medium tracking-wide text-teal-700 dark:text-teal-400">
          {status}
        </p>
        <h1 className="mt-3 text-4xl font-semibold tracking-tight text-zinc-900 dark:text-zinc-50">
          {title}
        </h1>
        <p className="mt-4 text-lg text-zinc-600 dark:text-zinc-400">{message}</p>
        <p className="mt-10">
          <Link
            href={route.home()}
            className="text-sm font-medium text-teal-800 hover:text-teal-950 dark:text-teal-300 dark:hover:text-teal-200"
          >
            回到首页
          </Link>
        </p>
      </div>
    </main>
  )
}
