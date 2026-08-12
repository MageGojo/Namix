import type { DemoPage } from '../generated/DemoPage'
import { route } from '../routes'
import type { PageProps } from '../types'

type Props = PageProps<DemoPage>

function pageHref(page: number): string {
  return page <= 1 ? route.island() : `${route.island()}?page=${page}`
}

export default function Island({
  title,
  page,
  perPage,
  total,
  totalPages,
  items,
}: Props) {
  const from = (page - 1) * perPage + 1
  const to = Math.min(page * perPage, total)

  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-sky-50/40">
      <div className="mx-auto max-w-3xl px-6 py-14">
        <a
          href={route.home()}
          className="text-sm font-medium tracking-wide text-sky-700 hover:text-sky-900"
        >
          ← Namix
        </a>

        <header className="mt-8">
          <p className="text-xs font-medium uppercase tracking-[0.2em] text-sky-700/80">
            mode = island
          </p>
          <h1 className="mt-2 text-3xl font-semibold tracking-tight text-zinc-900">{title}</h1>
          <p className="mt-2 text-sm text-zinc-500">
            共 {total} 条 · 每页 {perPage} 条 · 当前第 {page}/{totalPages} 页（显示 {from}–{to}）
          </p>
          <p className="mt-1 text-sm text-zinc-400">
            Island：可选 SSR HTML + 内联{' '}
            <code className="rounded bg-zinc-200/70 px-1">__namix_page</code>；有正文时 hydrate，
            否则 mount
          </p>
        </header>

        <ol className="mt-10 divide-y divide-zinc-200 border-y border-zinc-200">
          {items.map((item) => (
            <li key={item.id} className="flex gap-4 py-4">
              <span className="w-12 shrink-0 text-sm font-medium text-sky-700">{item.id}</span>
              <div>
                <h2 className="text-base font-medium text-zinc-900">{item.title}</h2>
                <p className="mt-1 text-sm text-zinc-500">{item.summary}</p>
              </div>
            </li>
          ))}
        </ol>

        <nav className="mt-8 flex flex-wrap items-center justify-between gap-3 text-sm" aria-label="分页">
          {page > 1 ? (
            <a
              href={pageHref(page - 1)}
              className="rounded-lg border border-zinc-300 bg-white px-3 py-1.5 text-zinc-700 hover:border-sky-600 hover:text-sky-800"
            >
              ← 上一页
            </a>
          ) : (
            <span className="px-3 py-1.5 text-zinc-300">← 上一页</span>
          )}

          <div className="flex flex-wrap gap-1">
            {Array.from({ length: totalPages }, (_, i) => i + 1).map((p) => (
              <a
                key={p}
                href={pageHref(p)}
                className={
                  p === page
                    ? 'rounded-lg bg-sky-700 px-2.5 py-1.5 font-medium text-white'
                    : 'rounded-lg px-2.5 py-1.5 text-zinc-600 hover:bg-white hover:text-sky-800'
                }
                aria-current={p === page ? 'page' : undefined}
              >
                {p}
              </a>
            ))}
          </div>

          {page < totalPages ? (
            <a
              href={pageHref(page + 1)}
              className="rounded-lg border border-zinc-300 bg-white px-3 py-1.5 text-zinc-700 hover:border-sky-600 hover:text-sky-800"
            >
              下一页 →
            </a>
          ) : (
            <span className="px-3 py-1.5 text-zinc-300">下一页 →</span>
          )}
        </nav>
      </div>
    </main>
  )
}
