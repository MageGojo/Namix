import { useEffect, useRef, useState } from 'react'
import { AppNav } from '../components/nav'
import type { ChatPage } from '../generated/ChatPage'
import { Head, useChatChannel } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<ChatPage>

export default function Chat({ title, me: pageMe }: Props) {
  const { me, status, users, lines, send } = useChatChannel(pageMe)
  const [text, setText] = useState('')
  const bottomRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [lines])

  const statusLabel =
    status === 'open' ? '已连接' : status === 'connecting' ? '连接中…' : '已断开'

  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-teal-50/40">
      <Head title={`${title} · Namix`} />
      <div className="mx-auto flex h-screen max-w-2xl flex-col px-4 py-6 sm:px-6">
        <AppNav username={me.username} />
        <header className="mb-3 flex flex-wrap items-end justify-between gap-2">
          <div>
            <h1 className="text-2xl font-semibold tracking-tight text-zinc-900">{title}</h1>
            <p className="mt-1 text-sm text-zinc-500">
              你是 <b className="text-zinc-800">{me.username}</b>
              <span className="text-zinc-400"> #{me.id}</span> · {statusLabel}
            </p>
          </div>
          <p className="text-xs text-zinc-500">
            在线 {users.length}：
            {users.length
              ? users.map((u) => `${u.username}(#${u.id})`).join('、')
              : '—'}
          </p>
        </header>

        <section className="flex min-h-0 flex-1 flex-col rounded-2xl border border-zinc-200/80 bg-white shadow-sm">
          <div className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
            {lines.length === 0 ? (
              <p className="text-center text-sm text-zinc-400">还没有消息，打个招呼吧</p>
            ) : null}
            {lines.map((line, i) =>
              line.kind === 'system' ? (
                <p key={i} className="text-center text-xs text-zinc-400">
                  {line.text}
                </p>
              ) : (
                <div
                  key={i}
                  className={
                    line.self ? 'flex flex-col items-end' : 'flex flex-col items-start'
                  }
                >
                  <span className="mb-0.5 text-xs text-zinc-400">
                    {line.message.username}
                  </span>
                  <div
                    className={
                      line.self
                        ? 'max-w-[85%] rounded-2xl rounded-br-md bg-teal-700 px-3 py-2 text-sm text-white'
                        : 'max-w-[85%] rounded-2xl rounded-bl-md bg-zinc-100 px-3 py-2 text-sm text-zinc-800'
                    }
                  >
                    {line.message.text}
                  </div>
                </div>
              ),
            )}
            <div ref={bottomRef} />
          </div>

          <form
            className="flex gap-2 border-t border-zinc-100 p-3"
            onSubmit={(e) => {
              e.preventDefault()
              if (send(text)) setText('')
            }}
          >
            <input
              value={text}
              onChange={(e) => setText(e.target.value)}
              placeholder={status === 'open' ? '输入消息…' : '等待连接…'}
              disabled={status !== 'open'}
              className="min-w-0 flex-1 rounded-xl border border-zinc-300 px-3 py-2 text-sm outline-none ring-teal-600/30 focus:border-teal-600 focus:ring-2 disabled:bg-zinc-50"
              autoComplete="off"
            />
            <button
              type="submit"
              disabled={status !== 'open' || !text.trim()}
              className="rounded-xl bg-teal-700 px-4 py-2 text-sm font-medium text-white hover:bg-teal-800 disabled:opacity-50"
            >
              发送
            </button>
          </form>
        </section>
      </div>
    </main>
  )
}
