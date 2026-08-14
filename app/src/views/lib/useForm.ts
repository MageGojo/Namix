import { useCallback, useRef, useState, type FormEvent } from 'react'
import { ActionException, parseActionFailure, type FieldErrors } from './actionError'
import { t } from './i18n'
import { router, type VisitOptions } from './router'

type ActionResult = {
  redirect?: string
  errors?: FieldErrors
  message?: string
  error?: string
} & Record<string, unknown>

export type SubmitOpts<TResult = ActionResult> = VisitOptions & {
  /** 成功回调（在自动跳转之前） */
  onSuccess?: (result: TResult) => void | Promise<void>
  /** 失败回调；可在此改文案 / toast */
  onError?: (errors: FieldErrors, err: ActionException) => void
  /**
   * 自定义字段错误映射（在 `t()` / `messages` 翻译之前）。
   * 收到的是稳定码，返回值仍应是码。
   */
  mapErrors?: (errors: FieldErrors, err: ActionException) => FieldErrors
  /** 按稳定码覆盖文案，例如 `{ 'username.taken': '换一个用户名' }` */
  messages?: Record<string, string>
  /** 收到 redirect 时是否软跳转（默认 true） */
  followRedirect?: boolean
  /** 失败时是否继续向外抛（默认 false，便于 UI 展示） */
  throwOnError?: boolean
}

/**
 * Inertia 风格 `useForm`，对接 `#[server]` / callRust 的 `{ redirect }` / `{ errors }`：
 *
 * ```ts
 * const form = useForm({ username: '', password: '' })
 * <form onSubmit={form.onSubmit(login, {
 *   messages: { 'auth.failed': '账号或密码不对' },
 *   mapErrors: (e) => ({ ...e, password: e.password ?? e._ }),
 * })}>
 * ```
 */
export function useForm<T extends Record<string, unknown>>(initial: T) {
  const defaults = useRef(initial)
  const [data, setDataState] = useState<T>(initial)
  const [errors, setErrors] = useState<FieldErrors>({})
  const [processing, setProcessing] = useState(false)
  const [wasSuccessful, setWasSuccessful] = useState(false)
  const [recentlySuccessful, setRecentlySuccessful] = useState(false)
  const [response, setResponse] = useState<unknown>(null)
  const successTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  const setData = useCallback(
    ((key: keyof T | Partial<T> | ((prev: T) => T), value?: unknown) => {
      if (typeof key === 'function') {
        setDataState(key)
      } else if (typeof key === 'object' && key !== null) {
        setDataState((prev) => ({ ...prev, ...key }))
      } else {
        setDataState((prev) => ({ ...prev, [key]: value as T[keyof T] }))
      }
    }) as {
      (key: keyof T, value: T[keyof T]): void
      (values: Partial<T>): void
      (updater: (prev: T) => T): void
    },
    [],
  )

  function clearErrors(...fields: string[]) {
    if (fields.length === 0) {
      setErrors({})
      return
    }
    setErrors((prev) => {
      const next = { ...prev }
      for (const f of fields) delete next[f]
      return next
    })
  }

  function setError(field: string, message: string) {
    setErrors((prev) => ({ ...prev, [field]: message }))
  }

  function error(field: string): string | undefined {
    return errors[field]
  }

  function reset(...fields: (keyof T)[]) {
    if (fields.length === 0) {
      setDataState(defaults.current)
      return
    }
    setDataState((prev) => {
      const next = { ...prev }
      for (const f of fields) next[f] = defaults.current[f]
      return next
    })
  }

  function setDefaults(next?: T) {
    defaults.current = next ?? data
  }

  function applyFailure(err: unknown, opts: SubmitOpts) {
    const ex = err instanceof ActionException ? err : parseActionFailure(err)
    let bag = { ...ex.errors }
    if (opts.mapErrors) bag = opts.mapErrors(bag, ex)
    const resolved: FieldErrors = {}
    for (const [field, code] of Object.entries(bag)) {
      resolved[field] = opts.messages?.[code] ?? t(code)
    }
    setErrors(resolved)
    setResponse(ex.response)
    opts.onError?.(resolved, ex)
    return ex
  }

  async function submit<TResult extends ActionResult = ActionResult>(
    action: (input: T) => Promise<TResult | void>,
    opts: SubmitOpts<TResult> = {},
  ) {
    setProcessing(true)
    setWasSuccessful(false)
    setRecentlySuccessful(false)
    setResponse(null)
    clearErrors()
    try {
      const result = ((await action(data)) ?? {}) as TResult

      // 少数 action 用 200 + errors 表达失败
      if (result.errors && Object.keys(result.errors).length > 0) {
        const ex = new ActionException(
          result.error || result.message || '校验失败',
          result.errors,
          result,
        )
        applyFailure(ex, opts as SubmitOpts)
        if (opts.throwOnError) throw ex
        return result
      }

      setWasSuccessful(true)
      setRecentlySuccessful(true)
      setResponse(result)
      if (successTimer.current) clearTimeout(successTimer.current)
      successTimer.current = setTimeout(() => setRecentlySuccessful(false), 2000)
      await opts.onSuccess?.(result)
      if (opts.followRedirect !== false && typeof result.redirect === 'string') {
        await router.visit(result.redirect, {
          replace: opts.replace ?? true,
          preserveScroll: opts.preserveScroll,
          showProgress: opts.showProgress,
        })
      }
      return result
    } catch (err) {
      const ex = applyFailure(err, opts as SubmitOpts)
      if (opts.throwOnError) throw ex
      return undefined
    } finally {
      setProcessing(false)
    }
  }

  /** `<form onSubmit={form.onSubmit(login, { messages: … })}>` */
  function onSubmit<TResult extends ActionResult = ActionResult>(
    action: (input: T) => Promise<TResult | void>,
    opts?: SubmitOpts<TResult>,
  ) {
    return (e: FormEvent) => {
      e.preventDefault()
      void submit(action, opts)
    }
  }

  async function get(url: string, opts?: VisitOptions) {
    setProcessing(true)
    try {
      const u = new URL(url, window.location.href)
      for (const [k, v] of Object.entries(data)) {
        if (v === undefined || v === null || v === '') continue
        u.searchParams.set(k, String(v))
      }
      await router.visit(`${u.pathname}${u.search}`, opts)
    } finally {
      setProcessing(false)
    }
  }

  return {
    data,
    setData,
    errors,
    error,
    setError,
    processing,
    wasSuccessful,
    recentlySuccessful,
    /** 最近一次成功/失败原始响应 */
    response,
    submit,
    onSubmit,
    get,
    reset,
    clearErrors,
    setDefaults,
    hasErrors: Object.keys(errors).length > 0,
  }
}
