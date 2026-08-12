import { AppNav } from '../components/nav'
import type { MePage } from '../generated/MePage'
import { ProfileForm } from '../generated/fields'
import { CsrfField, Head, router, usePage } from '../namix'
import { route } from '../routes'
import type { PageProps } from '../types'

type Props = PageProps<MePage>

export default function Me({
  title,
  username,
  userId,
  displayName,
  email,
  bio,
  error,
  saved,
}: Props) {
  const page = usePage<MePage>()

  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/30">
      <Head title={`${title} · Namix`} />
      <div className="mx-auto max-w-xl px-6 py-14">
        <AppNav username={username} />
        <h1 className="text-3xl font-semibold tracking-tight text-zinc-900">{title}</h1>
        <p className="mt-1 text-xs text-zinc-400">
          usePage · {page.component} ·{' '}
          <button
            type="button"
            className="underline hover:text-teal-700"
            onClick={() => void router.reload()}
          >
            reload
          </button>
        </p>
        <p className="mt-2 text-sm text-zinc-500">
          账号 <b className="text-zinc-800">{username}</b>（id={userId}）· User ↔ Profile 1:1
        </p>

        {error ? (
          <p className="mt-4 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-700">{error}</p>
        ) : null}
        {saved ? (
          <p className="mt-4 rounded-lg bg-teal-50 px-3 py-2 text-sm text-teal-800">已保存</p>
        ) : null}

        <form method="post" action={route.me.submit()} className="mt-8 space-y-4">
          <CsrfField />
          <label className="block space-y-1.5">
            <span className="text-sm font-medium text-zinc-700">显示名</span>
            <input
              name={ProfileForm.DisplayName}
              defaultValue={displayName}
              className="w-full rounded-lg border border-zinc-300 px-3 py-2 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2"
            />
          </label>
          <label className="block space-y-1.5">
            <span className="text-sm font-medium text-zinc-700">邮箱</span>
            <input
              name={ProfileForm.Email}
              defaultValue={email}
              className="w-full rounded-lg border border-zinc-300 px-3 py-2 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2"
            />
          </label>
          <label className="block space-y-1.5">
            <span className="text-sm font-medium text-zinc-700">简介</span>
            <textarea
              name={ProfileForm.Bio}
              rows={4}
              defaultValue={bio}
              className="w-full rounded-lg border border-zinc-300 px-3 py-2 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2"
            />
          </label>
          <button
            type="submit"
            className="rounded-lg bg-teal-700 px-4 py-2.5 text-sm font-medium text-white hover:bg-teal-800"
          >
            保存
          </button>
        </form>
      </div>
    </main>
  )
}
