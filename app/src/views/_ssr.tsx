/**
 * Namix SSR worker entry — Vite: `vite build --ssr src/views/_ssr.tsx`
 * Protocol: one JSON line in → one JSON line out `{ ok, html }` / `{ ok:false, error }`.
 */
import { createElement } from 'react'
import { renderToString } from 'react-dom/server'
import { createInterface } from 'node:readline'
import { pages } from './generated/registry'

export type SsrPayload = {
  component: string
  props: Record<string, unknown>
  url: string
}

export function renderPage(payload: SsrPayload): string {
  const Comp = pages[payload.component]
  if (!Comp) {
    return `<main class="mx-auto max-w-xl px-6 py-16"><h1 class="text-2xl font-semibold">Unknown view</h1><p class="mt-2 text-zinc-600">${escapeHtml(payload.component)}</p></main>`
  }
  return renderToString(
    createElement(Comp, { ...payload.props, url: payload.url }),
  )
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

function reply(obj: unknown) {
  process.stdout.write(`${JSON.stringify(obj)}\n`)
}

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity })
rl.on('line', (line) => {
  const trimmed = line.trim()
  if (!trimmed) return
  try {
    const payload = JSON.parse(trimmed) as SsrPayload
    if (!payload.component) {
      reply({ ok: false, error: 'missing component' })
      return
    }
    const html = renderPage({
      component: payload.component,
      props: payload.props ?? {},
      url: payload.url ?? '/',
    })
    reply({ ok: true, html })
  } catch (err) {
    reply({
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    })
  }
})

rl.on('close', () => {
  process.exit(0)
})
