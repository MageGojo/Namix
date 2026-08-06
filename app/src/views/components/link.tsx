import {
  forwardRef,
  useRef,
  type AnchorHTMLAttributes,
  type MouseEvent,
} from 'react'
import { router, type VisitOptions } from '../lib/router'

export type LinkProps = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, 'href'> &
  VisitOptions & {
    href: string
    /**
     * 悬停预取（Inertia）：`true` ≈ 75ms 后 prefetch；
     * 也可传毫秒数自定义延迟。
     */
    prefetch?: boolean | number
  }

/**
 * Inertia 风格 `<Link>`：同站 GET 走 `X-Namix-Props` 软导航 + 顶栏进度条。
 * Ctrl/⌘ 点击、`target="_blank"`、外链仍走浏览器默认行为。
 */
export const Link = forwardRef<HTMLAnchorElement, LinkProps>(function Link(
  {
    href,
    replace,
    preserveScroll,
    showProgress,
    prefetch,
    onClick,
    onMouseEnter,
    onMouseLeave,
    onFocus,
    target,
    ...rest
  },
  ref,
) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  function clearPrefetchTimer() {
    if (timer.current) {
      clearTimeout(timer.current)
      timer.current = null
    }
  }

  function schedulePrefetch() {
    if (!prefetch) return
    clearPrefetchTimer()
    const delay = typeof prefetch === 'number' ? prefetch : 75
    timer.current = setTimeout(() => {
      void router.prefetch(href)
    }, delay)
  }

  function handleClick(e: MouseEvent<HTMLAnchorElement>) {
    onClick?.(e)
    if (e.defaultPrevented) return
    if (target && target !== '_self') return
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
    e.preventDefault()
    void router.visit(href, { replace, preserveScroll, showProgress })
  }

  return (
    <a
      ref={ref}
      href={href}
      target={target}
      onClick={handleClick}
      onMouseEnter={(e) => {
        onMouseEnter?.(e)
        schedulePrefetch()
      }}
      onMouseLeave={(e) => {
        onMouseLeave?.(e)
        clearPrefetchTimer()
      }}
      onFocus={(e) => {
        onFocus?.(e)
        schedulePrefetch()
      }}
      {...rest}
    />
  )
})
