import { AppNav } from '../components/nav'
import type { ProfilePage } from '../generated/ProfilePage'
import type { PageProps } from '../types'

type Props = PageProps<ProfilePage>

export default function Profile({
  title,
  displayName,
  username,
  email,
  bio,
  viewer,
  postTitles,
}: Props) {
  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/30">
      <div className="mx-auto max-w-xl px-6 py-14">
        <AppNav username={viewer} />
        <h1 className="text-3xl font-semibold tracking-tight text-zinc-900">
          {displayName || title}
        </h1>
        <p className="mt-2 text-sm text-zinc-500">
          @{username}
          {email ? ` · ${email}` : ''}
        </p>
        {bio ? <p className="mt-4 text-zinc-700">{bio}</p> : null}

        <h2 className="mt-10 text-lg font-medium text-zinc-900">文章（{postTitles.length}）</h2>
        <ul className="mt-3 list-disc space-y-1 pl-5 text-sm text-zinc-700">
          {postTitles.map((t, i) => (
            <li key={`${t}-${i}`}>{t}</li>
          ))}
        </ul>
        <p className="mt-8 text-sm text-zinc-400">查看者：{viewer}</p>
      </div>
    </main>
  )
}
