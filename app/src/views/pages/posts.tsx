import { AppNav } from '../components/nav'
import type { PostsPage } from '../generated/PostsPage'
import { PostForm } from '../generated/fields'
import { CsrfField, route } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<PostsPage>

export default function Posts({ title, username, error, items = [], csrfToken }: Props) {
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
          <CsrfField token={csrfToken} />
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
          {items.map((p) => (
            <article key={p.id} className="space-y-3 py-4">
              <form method="post" action={route.posts.update({ id: p.id })} className="space-y-3">
                <CsrfField token={csrfToken} />
                <label className="block space-y-1.5">
                  <span className="text-sm font-medium text-zinc-700">标题</span>
                  <input
                    name={PostForm.Title}
                    defaultValue={p.title}
                    className="w-full rounded-lg border border-zinc-300 px-3 py-2 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2"
                  />
                </label>
                <label className="block space-y-1.5">
                  <span className="text-sm font-medium text-zinc-700">正文</span>
                  <textarea
                    name={PostForm.Body}
                    rows={3}
                    defaultValue={p.body}
                    className="w-full rounded-lg border border-zinc-300 px-3 py-2 outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2"
                  />
                </label>
                <div className="flex flex-wrap gap-2">
                  <button
                    type="submit"
                    className="rounded-lg bg-zinc-800 px-3 py-2 text-sm font-medium text-white hover:bg-zinc-900"
                  >
                    保存
                  </button>
                </div>
              </form>
              <form method="post" action={route.posts.destroy({ id: p.id })}>
                <CsrfField token={csrfToken} />
                <button
                  type="submit"
                  className="rounded-lg border border-red-200 px-3 py-2 text-sm font-medium text-red-700 hover:bg-red-50"
                >
                  删除
                </button>
              </form>
            </article>
          ))}
        </div>
      </div>
    </main>
  )
}
