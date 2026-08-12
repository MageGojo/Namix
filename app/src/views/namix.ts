/**
 * Namix 前端（Inertia / Laravel 风格常用 API）
 *
 * ```ts
 * import { Link, Head, useForm, usePage, router } from '../namix'
 * import { route } from '../routes'
 * ```
 */
export { Link, type LinkProps } from './components/link'
export { Head } from './components/head'
export { router, type VisitOptions } from './lib/router'
export { useForm, type SubmitOpts } from './lib/useForm'
export { csrfToken, CsrfField, type CsrfFieldProps } from './lib/csrf'
export {
  ActionException,
  parseActionFailure,
  translateErrors,
  type FieldErrors,
} from './lib/actionError'
export { usePage, PageProvider } from './lib/page'
export { useChatChannel, type ChatLine } from './lib/useChatChannel'
export { progress, configureProgress } from './lib/progress'
export type { NamixPage, PageProps } from './types'
