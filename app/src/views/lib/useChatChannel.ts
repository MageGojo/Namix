import { useEffect, useRef, useState } from 'react'
import type { ChatMessage } from '../generated/ChatMessage'
import type { ChatUser } from '../generated/ChatUser'
import { route } from '../namix'

export type ChatLine =
  | { kind: 'chat'; message: ChatMessage; self: boolean }
  | { kind: 'system'; text: string }

type ServerEnvelope =
  | { type: 'hello'; me: ChatUser }
  | ({ type: 'chat' } & ChatMessage)
  | { type: 'system'; text: string }
  | { type: 'presence'; users: ChatUser[] }

function wsUrl(path: string): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${proto}//${location.host}${path}`
}

/**
 * Laravel Echo 风格：连命名路由 `ws.chat`，用 `me.id` 识别自己。
 * 身份以服务端 `hello` 为准，props.me 仅作首屏占位。
 */
export function useChatChannel(pageMe: ChatUser) {
  const [me, setMe] = useState(pageMe)
  const [status, setStatus] = useState<'connecting' | 'open' | 'closed'>('connecting')
  const [users, setUsers] = useState<ChatUser[]>([])
  const [lines, setLines] = useState<ChatLine[]>([])
  const wsRef = useRef<WebSocket | null>(null)
  const meIdRef = useRef(pageMe.id)

  useEffect(() => {
    meIdRef.current = me.id
  }, [me.id])

  useEffect(() => {
    const ws = new WebSocket(wsUrl(route.ws.chat()))
    wsRef.current = ws
    setStatus('connecting')

    ws.onopen = () => setStatus('open')
    ws.onclose = () => setStatus('closed')
    ws.onerror = () => setStatus('closed')
    ws.onmessage = (ev) => {
      let msg: ServerEnvelope
      try {
        msg = JSON.parse(String(ev.data)) as ServerEnvelope
      } catch {
        setLines((prev) => [...prev, { kind: 'system', text: String(ev.data) }])
        return
      }

      switch (msg.type) {
        case 'hello':
          setMe(msg.me)
          meIdRef.current = msg.me.id
          break
        case 'chat': {
          const message: ChatMessage = {
            userId: msg.userId,
            username: msg.username,
            text: msg.text,
            at: msg.at,
          }
          setLines((prev) => [
            ...prev,
            { kind: 'chat', message, self: message.userId === meIdRef.current },
          ])
          break
        }
        case 'system':
          setLines((prev) => [...prev, { kind: 'system', text: msg.text }])
          break
        case 'presence':
          setUsers(msg.users)
          break
      }
    }

    return () => {
      ws.close()
      wsRef.current = null
    }
  }, [pageMe.id])

  function send(text: string) {
    const body = text.trim()
    if (!body || wsRef.current?.readyState !== WebSocket.OPEN) return false
    wsRef.current.send(JSON.stringify({ type: 'chat', text: body }))
    return true
  }

  return { me, status, users, lines, send }
}
