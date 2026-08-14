export type FieldErrors = Record<string, string>

/** Server Action 422：`{ error, message, errors }` */
export class ActionException extends Error {
  readonly errors: FieldErrors
  readonly response: unknown

  constructor(message: string, errors: FieldErrors = {}, response?: unknown) {
    super(message)
    this.name = 'ActionException'
    this.errors = errors
    this.response = response
  }
}

function firstMsg(v: unknown): string | null {
  if (typeof v === 'string' && v) return v
  if (Array.isArray(v) && typeof v[0] === 'string') return v[0]
  return null
}

/** 把后端 `errors` 规范成 `{ field: string }` */
export function normalizeErrors(raw: unknown): FieldErrors {
  const out: FieldErrors = {}
  if (!raw || typeof raw !== 'object') return out
  for (const [k, v] of Object.entries(raw as Record<string, unknown>)) {
    const msg = firstMsg(v)
    if (msg) out[k] = msg
  }
  return out
}

/** 从 callRust / nx_call 抛出的字符串或 Error 解析结构化错误 */
export function parseActionFailure(err: unknown): ActionException {
  const text =
    typeof err === 'string'
      ? err
      : err instanceof Error
        ? err.message
        : String(err)

  const trimmed = text.trim()
  if (trimmed.startsWith('{')) {
    try {
      const json = JSON.parse(trimmed) as {
        error?: string
        message?: string
        errors?: unknown
      }
      const errors = normalizeErrors(json.errors)
      const message = json.error || json.message || '请求失败'
      if (Object.keys(errors).length === 0) {
        errors._ = message
      }
      return new ActionException(message, errors, json)
    } catch {
      /* fall through */
    }
  }

  return new ActionException(trimmed || '请求失败', { _: trimmed || '请求失败' })
}

/** 按稳定码替换；未命中则保留原值（再交给 `t()`） */
export function translateErrors(
  errors: FieldErrors,
  messages: Record<string, string>,
): FieldErrors {
  const out: FieldErrors = {}
  for (const [k, v] of Object.entries(errors)) {
    out[k] = messages[v] ?? v
  }
  return out
}
