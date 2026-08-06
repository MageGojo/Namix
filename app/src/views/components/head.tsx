import { useEffect, type ReactNode } from 'react'

type HeadProps = {
  title?: string
  children?: ReactNode
}

/**
 * Inertia `<Head>`：同步 `document.title`；子节点里可用 `<meta name="…" content="…" />`。
 */
export function Head({ title, children }: HeadProps) {
  useEffect(() => {
    if (title) document.title = title
  }, [title])

  useEffect(() => {
    if (!children) return
    const nodes = Array.isArray(children) ? children : [children]
    const applied: HTMLMetaElement[] = []
    for (const node of nodes) {
      if (!node || typeof node !== 'object' || !('props' in node)) continue
      const props = (node as { props?: { name?: string; content?: string; property?: string } })
        .props
      if (!props?.content) continue
      const key = props.name ? `name` : props.property ? `property` : null
      const val = props.name ?? props.property
      if (!key || !val) continue
      let el = document.head.querySelector(`meta[${key}="${CSS.escape(val)}"]`) as
        | HTMLMetaElement
        | null
      if (!el) {
        el = document.createElement('meta')
        el.setAttribute(key, val)
        document.head.appendChild(el)
        applied.push(el)
      }
      el.content = props.content
    }
    return () => {
      for (const el of applied) el.remove()
    }
  }, [children])

  return null
}
