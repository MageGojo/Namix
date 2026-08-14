import { AppNav } from '../components/nav'
import { DataTable } from '../components/data-table'
import type { AdminUsersPage } from '../generated/AdminUsersPage'
import { Head } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<AdminUsersPage>

export default function AdminUsers({
  title,
  rows,
  total,
  perPage,
  currentPage,
  lastPage,
  from,
  to,
}: Props) {
  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/30">
      <Head title={`${title} · Namix`} />
      <div className="mx-auto max-w-3xl px-6 py-14">
        <AppNav />
        <h1 className="text-3xl font-semibold tracking-tight text-zinc-900">{title}</h1>
        <p className="mt-2 text-sm text-zinc-500">角色与权限在服务端判定；本表只展示结果。</p>
        <div className="mt-8">
          <DataTable
            columns={[
              { key: 'id', label: 'ID' },
              { key: 'username', label: '用户名' },
              { key: 'role', label: '角色' },
              { key: 'vip', label: 'VIP' },
              { key: 'verified', label: '邮箱' },
            ]}
            rows={rows as Array<Record<string, unknown>>}
            page={{ total, perPage, currentPage, lastPage, from, to }}
          />
        </div>
      </div>
    </main>
  )
}
