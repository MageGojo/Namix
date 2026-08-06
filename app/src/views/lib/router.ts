import type { NamixPage } from '../types'
import { progress } from './progress'

export type VisitOptions = {
  replace?: boolean
  preserveScroll?: boolean
  /** 不改 history（浏览器前进/后退） */
  history?: 'push' | 'replace' | 'none'
  showProgress?: boolean
  /** 强制绕过预取缓存 */
  fresh?: boolean
  onStart?: () => void
  onFinish?: () => void
  onSuccess?: (page: NamixPage) => void
}

type Listener = (page: NamixPage) => void
type EventName = 'start' | 'finish' | 'navigate'
type EventFn = (payload?: unknown) => void

const listeners = new Set<Listener>()
const events = new Map<EventName, Set<EventFn>>()
let navigating = false
let currentUrl =
  typeof window !== 'undefined' ? `${location.pathname}${location.search}` : '/'

type CacheEntry = { page: NamixPage; at: number }
const prefetchCache = new Map<string, CacheEntry>()
const PREFETCH_TTL = 30_000

function emit(name: EventName, payload?: unknown) {
  const set = events.get(name)
  if (!set) return
  for (const fn of set) fn(payload)
}

function notify(page: NamixPage) {
  currentUrl = page.url
  for (const fn of listeners) fn(page)
  emit('navigate', page)
}

function cacheKey(href: string): string {
  const url = new URL(href, window.location.href)
  return `${url.pathname}${url.search}`
}

async function fetchPage(href: string): Promise<NamixPage> {
  const url = new URL(href, window.location.href)
  const res = await fetch(`${url.pathname}${url.search}`, {
    headers: {
      Accept: 'application/vnd.namix.props+json, application/json',
      'X-Namix-Props': '1',
      'X-Requested-With': 'XMLHttpRequest',
    },
    credentials: 'same-origin',
    redirect: 'follow',
  })
  const ct = res.headers.get('content-type') || ''
  if (!res.ok || !ct.includes('json')) {
    throw new Error(`soft-nav ${res.status}`)
  }
  const page = (await res.json()) as NamixPage
  const next = page.url || `${url.pathname}${url.search}`
  return {
    component: page.component,
    props: page.props,
    url: next,
  }
}

function readCache(key: string): NamixPage | null {
  const hit = prefetchCache.get(key)
  if (!hit) return null
  if (Date.now() - hit.at > PREFETCH_TTL) {
    prefetchCache.delete(key)
    return null
  }
  return hit.page
}

async function swap(href: string, opts: VisitOptions = {}) {
  const url = new URL(href, window.location.href)
  if (url.origin !== window.location.origin) {
    window.location.href = url.href
    return
  }

  const key = cacheKey(url.href)
  const showProgress = opts.showProgress !== false
  opts.onStart?.()
  emit('start')
  if (showProgress) progress.start()
  navigating = true
  try {
    let page = !opts.fresh ? readCache(key) : null
    if (!page) {
      page = await fetchPage(url.href)
      prefetchCache.set(key, { page, at: Date.now() })
    }
    const mode = opts.history ?? (opts.replace ? 'replace' : 'push')
    if (mode === 'push') {
      history.pushState({ namix: true }, '', page.url)
    } else if (mode === 'replace') {
      history.replaceState({ namix: true }, '', page.url)
    }
    if (!opts.preserveScroll) {
      window.scrollTo(0, 0)
    }
    notify(page)
    opts.onSuccess?.(page)
  } catch {
    window.location.href = url.href
  } finally {
    navigating = false
    if (showProgress) progress.done()
    opts.onFinish?.()
    emit('finish')
  }
}

export const router = {
  visit(href: string, opts?: VisitOptions) {
    return swap(href, opts)
  },

  /** 重新拉取当前 URL props（Inertia `router.reload`） */
  reload(opts?: VisitOptions) {
    return swap(currentUrl, {
      ...opts,
      history: 'none',
      preserveScroll: opts?.preserveScroll ?? true,
      fresh: true,
    })
  },

  get(href: string, opts?: VisitOptions) {
    return swap(href, opts)
  },

  /** 悬停预取；成功写入短 TTL 缓存，点击时秒开 */
  async prefetch(href: string) {
    if (typeof window === 'undefined') return
    const url = new URL(href, window.location.href)
    if (url.origin !== window.location.origin) return
    const key = cacheKey(url.href)
    if (readCache(key)) return
    try {
      const page = await fetchPage(url.href)
      prefetchCache.set(key, { page, at: Date.now() })
    } catch {
      /* ignore prefetch failures */
    }
  },

  listen(fn: Listener) {
    listeners.add(fn)
    return () => {
      listeners.delete(fn)
    }
  },

  on(event: EventName, fn: EventFn) {
    let set = events.get(event)
    if (!set) {
      set = new Set()
      events.set(event, set)
    }
    set.add(fn)
    return () => {
      set!.delete(fn)
    }
  },

  flushPrefetch(href?: string) {
    if (!href) {
      prefetchCache.clear()
      return
    }
    prefetchCache.delete(cacheKey(href))
  },
}

if (typeof window !== 'undefined') {
  window.addEventListener('popstate', () => {
    if (navigating) return
    void swap(`${location.pathname}${location.search}`, {
      history: 'none',
      preserveScroll: true,
      fresh: true,
    })
  })
}
