type Column = {
  key: string
  label: string
}

type PageMeta = {
  total: number
  perPage: number
  currentPage: number
  lastPage: number
  from: number
  to: number
}

type Props = {
  columns: Column[]
  rows: Array<Record<string, unknown>>
  page?: PageMeta
}

/** 后台表格第一版：吃分页元数据，不含授权字段。 */
export function DataTable({ columns, rows, page }: Props) {
  return (
    <div className="overflow-hidden rounded-xl border border-zinc-200 bg-white">
      <table className="w-full text-left text-sm">
        <thead className="bg-zinc-50 text-zinc-500">
          <tr>
            {columns.map((col) => (
              <th key={col.key} className="px-4 py-2.5 font-medium">
                {col.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-zinc-100">
          {rows.length === 0 ? (
            <tr>
              <td className="px-4 py-8 text-center text-zinc-400" colSpan={columns.length}>
                暂无数据
              </td>
            </tr>
          ) : (
            rows.map((row, index) => (
              <tr key={String(row.id ?? index)} className="text-zinc-800">
                {columns.map((col) => (
                  <td key={col.key} className="px-4 py-2.5 font-mono text-[13px]">
                    {row[col.key] == null ? '' : String(row[col.key])}
                  </td>
                ))}
              </tr>
            ))
          )}
        </tbody>
      </table>
      {page ? (
        <p className="border-t border-zinc-100 px-4 py-2 text-xs text-zinc-500">
          {page.from}–{page.to} / {page.total} · 第 {page.currentPage}/{page.lastPage} 页
        </p>
      ) : null}
    </div>
  )
}
