import NProgress from 'nprogress'

let configured = false
let delayTimer: ReturnType<typeof setTimeout> | null = null
let inflight = 0

/** Inertia 风格：延迟显示顶栏进度（默认 250ms）。 */
export function configureProgress(opts?: { delay?: number; color?: string }) {
  if (configured || typeof document === 'undefined') return
  configured = true
  NProgress.configure({
    showSpinner: false,
    trickleSpeed: 200,
    minimum: 0.08,
  })
  const color = opts?.color ?? '#0f766e'
  document.documentElement.style.setProperty('--namix-progress', color)
}

const delayMs = () => 250

export const progress = {
  start() {
    if (typeof window === 'undefined') return
    inflight += 1
    if (inflight === 1 && !delayTimer) {
      delayTimer = setTimeout(() => {
        delayTimer = null
        NProgress.start()
      }, delayMs())
    }
  },
  done() {
    if (typeof window === 'undefined') return
    inflight = Math.max(0, inflight - 1)
    if (inflight > 0) return
    if (delayTimer) {
      clearTimeout(delayTimer)
      delayTimer = null
      return
    }
    NProgress.done()
  },
}
