import {
  createContext,
  useContext,
  type ReactNode,
} from 'react'
import type { NamixPage } from '../types'

const PageContext = createContext<NamixPage | null>(null)

export function PageProvider({
  page,
  children,
}: {
  page: NamixPage
  children: ReactNode
}) {
  return <PageContext.Provider value={page}>{children}</PageContext.Provider>
}

/** Inertia `usePage()`：读当前页 component / props / url。 */
export function usePage<P = Record<string, unknown>>(): NamixPage<P> {
  const page = useContext(PageContext)
  if (!page) {
    throw new Error('usePage() 须在 NamixApp / PageProvider 内使用')
  }
  return page as NamixPage<P>
}
