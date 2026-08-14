/**
 * Namix 前端（Inertia / Laravel 风格常用 API）
 *
 * ```ts
 * import { Link, Head, useForm, usePage, router, route, AppRoute } from '../namix'
 * route.login()  ≡  route(AppRoute.Login)
 * ```
 */
export { Link, type LinkProps } from './components/link'
export { Head } from './components/head'
export { applyTheme, setTheme, theme, toggleTheme, type Theme } from './lib/theme'
export { router, type VisitOptions } from './lib/router'
export { useForm, type SubmitOpts } from './lib/useForm'
export { csrfToken, CsrfField, type CsrfFieldProps } from './lib/csrf'
export { t } from './lib/i18n'
export { DataTable } from './components/data-table'
export {
  ActionException,
  parseActionFailure,
  translateErrors,
  type FieldErrors,
} from './lib/actionError'
export { usePage, PageProvider } from './lib/page'
export { useChatChannel, type ChatLine } from './lib/useChatChannel'
export { progress, configureProgress } from './lib/progress'
export { route, AppRoute, routes, type RouteName } from './routes'
export type { NamixPage, PageProps } from './types'
