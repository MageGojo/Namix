import { StrictMode, useEffect, useState } from 'react'
import { createRoot, hydrateRoot } from 'react-dom/client'
import './app.css'
import { loadNamixPage, type NamixPage } from './types'
import { pages } from './generated/registry'
import { PageProvider } from './lib/page'
import { configureProgress } from './lib/progress'
import { router } from './lib/router'

function NamixApp({ initial }: { initial: NamixPage }) {
  const [page, setPage] = useState(initial)

  useEffect(() => router.listen(setPage), [])

  const Comp = pages[page.component]
  if (!Comp) {
    return (
      <main className="mx-auto max-w-xl px-6 py-16">
        <h1 className="text-2xl font-semibold">Unknown view</h1>
        <p className="mt-2 text-zinc-600">
          没有注册组件{' '}
          <code className="rounded bg-zinc-200 px-1">{page.component}</code>
        </p>
      </main>
    )
  }

  return (
    <PageProvider page={page}>
      <Comp {...page.props} url={page.url} />
    </PageProvider>
  )
}

async function boot() {
  const root = document.getElementById('app')
  if (!root) throw new Error('#app missing')

  // 纯 SSR 页不加载本入口；若误加载则不再二次挂载
  if (root.dataset.namixMode === 'ssr') return

  configureProgress()

  try {
    const page = await loadNamixPage()
    const tree = (
      <StrictMode>
        <NamixApp initial={page} />
      </StrictMode>
    )

    const shouldHydrate = root.dataset.namixMode === 'island' && root.hasChildNodes()
    if (shouldHydrate) {
      hydrateRoot(root, tree)
    } else {
      createRoot(root).render(tree)
    }
  } catch (err) {
    root.textContent = `Namix view error: ${err instanceof Error ? err.message : String(err)}`
    console.error(err)
  }
}

void boot()
