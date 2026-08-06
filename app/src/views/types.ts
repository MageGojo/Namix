export type NamixPage<P = Record<string, unknown>> = {
  component: string
  props: P
  url: string
}

/** 页面组件 props = 后端 ViewData/ViewProps + 框架注入的 url */
export type PageProps<T> = T & { url?: string }

/**
 * 加载本页 props：
 * - SSR：读内联 `#__namix_page`
 * - SPA：`#app[data-namix-key]` → `GET /__namix/props/:key`
 * 路由不在载荷里——用 `import { route } from './routes'`。
 */
export async function loadNamixPage<P = Record<string, unknown>>(): Promise<NamixPage<P>> {
  const inline = document.getElementById('__namix_page')
  if (inline?.textContent) {
    const data = JSON.parse(inline.textContent) as NamixPage<P>
    inline.remove()
    return {
      component: data.component,
      props: data.props,
      url: data.url || location.pathname,
    }
  }

  const el = document.getElementById('app')
  const component = el?.dataset.namixView
  const key = el?.dataset.namixKey
  if (!component || !key) {
    throw new Error('missing data-namix-view / data-namix-key on #app')
  }

  const res = await fetch(`/__namix/props/${encodeURIComponent(key)}`, {
    headers: {
      Accept: 'application/json',
      'X-Namix-Props': '1',
    },
    credentials: 'same-origin',
  })
  if (!res.ok) {
    throw new Error(`props ${res.status}`)
  }
  const data = (await res.json()) as NamixPage<P>
  delete el.dataset.namixKey
  return {
    component: data.component || component,
    props: data.props,
    url: data.url || location.pathname,
  }
}
