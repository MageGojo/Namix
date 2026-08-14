export type Theme = 'dark' | 'light' | 'system'

const COOKIE = 'namix_theme'

function readCookie(source: string): string {
  for (const part of source.split(';')) {
    const separator = part.indexOf('=')
    if (separator < 0 || part.slice(0, separator).trim() !== COOKIE) continue
    const value = part.slice(separator + 1).trim()
    try {
      return decodeURIComponent(value)
    } catch {
      return value
    }
  }
  return ''
}

/** Current preference from the readable `namix_theme` cookie. */
export function theme(cookieSource?: string): Theme {
  const source = cookieSource ?? (typeof document === 'undefined' ? '' : document.cookie)
  const value = readCookie(source)
  if (value === 'dark' || value === 'light' || value === 'system') return value
  return 'system'
}

export function applyTheme(next: Theme = theme()) {
  if (typeof document === 'undefined') return
  const dark =
    next === 'dark' ||
    (next !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches)
  const resolved = dark ? 'dark' : 'light'
  document.documentElement.setAttribute('data-theme', resolved)
  document.documentElement.style.colorScheme = resolved
}

/** Persist theme and update `<html>` immediately. Next full load uses the cookie. */
export function setTheme(next: Theme) {
  if (typeof document === 'undefined') return
  const secure = location.protocol === 'https:' ? '; Secure' : ''
  document.cookie = `${COOKIE}=${encodeURIComponent(next)}; Path=/; Max-Age=31536000; SameSite=Lax${secure}`
  applyTheme(next)
}

export function toggleTheme() {
  setTheme(document.documentElement.getAttribute('data-theme') === 'dark' ? 'light' : 'dark')
}
