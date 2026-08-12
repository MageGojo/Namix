import { useEffect, useState } from 'react'

const CSRF_COOKIE = 'namix_csrf'

/** Read Namix's readable double-submit CSRF cookie. */
export function csrfToken(cookieSource?: string): string {
  const source = cookieSource ?? (typeof document === 'undefined' ? '' : document.cookie)
  for (const part of source.split(';')) {
    const separator = part.indexOf('=')
    if (separator < 0 || part.slice(0, separator).trim() !== CSRF_COOKIE) continue
    const value = part.slice(separator + 1).trim()
    try {
      return decodeURIComponent(value)
    } catch {
      return value
    }
  }
  return ''
}

export type CsrfFieldProps = { token?: string }

/** Hidden field for classic browser POST forms protected by Namix CSRF. */
export function CsrfField({ token }: CsrfFieldProps) {
  const [value, setValue] = useState(token ?? '')
  useEffect(() => {
    setValue(token ?? csrfToken())
  }, [token])
  return <input type="hidden" name="_csrf" value={value} />
}
