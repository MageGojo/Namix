import { AppNav } from '../components/nav'
import type { PostsPage } from '../generated/PostsPage'
import { PostForm } from '../generated/fields'
import { route } from '../routes'
import type { PageProps } from '../types'

type Props = PageProps<PostsPage>

export default function Posts({ title, username, error, items = [] }: Props) {
  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/30">
      <div className="mx-auto max-w-xl px-6 py-14">
        <AppNav username={username} />
        <h1 className="text-3xl font-semibold tracking-tight text-zinc-900">{title}</h1>
        <p className="mt-2 text-sm text-zinc-500">User ↔ Post 1:N · 共 {items.length} 篇</p>

        {error ? (
          <p className="mt-4 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-700">{error}</p>
        ) : null}

        <form method="post" action={route.posts.submit()} className="mt-8 space-y-4">
          <label className="block space-y-1.5">
            <span className="text-sm font-medium text-zinc-700">标题</span>
            <input
              name={PostForm.Title}
              className="w-full rounded-lg border border-zinc-300 px-3 py-2 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2"
            />
          </label>
          <label className="block space-y-1.5">
            <span className="text-sm font-medium text-zinc-700">正文</span>
            <textarea
              name={PostForm.Body}
              rows={4}
              className="w-full rounded-lg border border-zinc-300 px-3 py-2 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2"
            />
          </label>
          <button
            type="submit"
            className="rounded-lg bg-teal-700 px-4 py-2.5 text-sm font-medium text-white hover:bg-teal-800"
          >
            发布
          </button>
        </form>

        <div className="mt-10 divide-y divide-zinc-200 border-t border-zinc-200">
          {items.map((p, i) => (
            <article key={`${p.title}-${i}`} className="py-4">
              <h2 className="text-lg font-medium text-zinc-900">{p.title}</h2>
              <p className="mt-1 whitespace-pre-wrap text-sm text-zinc-600">{p.body}</p>
            </article>
          ))}
        </div>
      </div>
    </main>
  )
}
