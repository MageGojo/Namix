import zh from '../../../lang/zh-CN.json'
import en from '../../../lang/en.json'

type Json = string | number | boolean | null | Json[] | { [key: string]: Json }

const catalogs: Record<string, Record<string, string>> = {
  'zh-CN': flatten('', zh as Json, {}),
  zh: flatten('', zh as Json, {}),
  en: flatten('', en as Json, {}),
}

function flatten(prefix: string, value: Json, out: Record<string, string>): Record<string, string> {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) {
      const next = prefix ? `${prefix}.${key}` : key
      flatten(next, child, out)
    }
    return out
  }
  if (prefix && (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean')) {
    out[prefix] = String(value)
  }
  return out
}

export function locale(): string {
  if (typeof document === 'undefined') return 'zh-CN'
  return document.documentElement.lang || 'zh-CN'
}

function catalog(): Record<string, string> {
  const current = locale()
  return catalogs[current] ?? catalogs['zh-CN'] ?? {}
}

function interpolate(template: string, params?: Record<string, string>): string {
  if (!params) return template
  let out = template
  for (const [key, value] of Object.entries(params)) {
    out = out.replaceAll(`:${key}`, value)
  }
  return out
}

/** Look up `auth.failed` / `username.taken`; falls back to `validation.{rule}`. */
export function t(key: string, params?: Record<string, string>): string {
  const messages = catalog()
  const specific = messages[key]
  if (specific) return interpolate(specific, params)
  const dot = key.lastIndexOf('.')
  if (dot > 0) {
    const attribute = key.slice(0, dot)
    const rule = key.slice(dot + 1)
    const fallback = messages[`validation.${rule}`]
    if (fallback) {
      const attr = messages[`attributes.${attribute}`] ?? attribute
      return interpolate(fallback, { attribute: attr, ...params })
    }
  }
  return interpolate(key, params)
}
