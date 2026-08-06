import { useState } from 'react'
import { AppNav } from '../components/nav'
import { mail_send } from '../generated/actions/mail_send'
import { mail_simulate_inbound } from '../generated/actions/mail_simulate_inbound'
import { sms_send_code } from '../generated/actions/sms_send_code'
import { sms_verify_code } from '../generated/actions/sms_verify_code'
import type { MailboxPage } from '../generated/MailboxPage'
import { Head, useForm } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<MailboxPage>

export default function Mailbox({
  title,
  username,
  mailFrom,
  mailDriver,
  smsDriver,
  outbox,
  inbox,
  smsSent,
}: Props) {
  const sendMail = useForm({ to: '', subject: '你好 from Namix', text: '这是一封测试邮件。' })
  const inbound = useForm({
    from: 'friend@example.com',
    subject: '入站测试',
    text: '模拟 webhook / IMAP 收取。',
  })
  const phoneForm = useForm({ phone: '13800138000', code: '' })
  const [tip, setTip] = useState<string | null>(null)

  return (
    <main className="min-h-screen bg-gradient-to-b from-zinc-50 to-sky-50/40">
      <Head title={`${title} · Namix`} />
      <div className="mx-auto max-w-3xl px-4 py-10 sm:px-6">
        <AppNav username={username} />
        <header className="mb-6">
          <h1 className="text-3xl font-semibold tracking-tight text-zinc-900">{title}</h1>
          <p className="mt-2 text-sm text-zinc-500">
            Mail <b className="text-zinc-700">{mailDriver}</b> · from {mailFrom} · SMS{' '}
            <b className="text-zinc-700">{smsDriver}</b>
          </p>
          {tip ? (
            <p className="mt-3 rounded-lg bg-teal-50 px-3 py-2 text-sm text-teal-800">{tip}</p>
          ) : null}
        </header>

        <div className="grid gap-6 lg:grid-cols-2">
          <section className="space-y-3 rounded-2xl border border-zinc-200 bg-white p-4 shadow-sm">
            <h2 className="text-sm font-semibold text-zinc-800">发送邮件</h2>
            <form
              className="space-y-2"
              onSubmit={sendMail.onSubmit(mail_send, {
                onSuccess: () => setTip('邮件已发送（见发件箱）'),
              })}
            >
              <input
                className="w-full rounded-lg border border-zinc-300 px-3 py-2 text-sm"
                placeholder="收件人"
                value={sendMail.data.to}
                onChange={(e) => sendMail.setData('to', e.target.value)}
              />
              <input
                className="w-full rounded-lg border border-zinc-300 px-3 py-2 text-sm"
                placeholder="主题"
                value={sendMail.data.subject}
                onChange={(e) => sendMail.setData('subject', e.target.value)}
              />
              <textarea
                className="w-full rounded-lg border border-zinc-300 px-3 py-2 text-sm"
                rows={3}
                value={sendMail.data.text}
                onChange={(e) => sendMail.setData('text', e.target.value)}
              />
              <button
                type="submit"
                disabled={sendMail.processing}
                className="rounded-lg bg-sky-700 px-3 py-2 text-sm font-medium text-white disabled:opacity-50"
              >
                发送
              </button>
            </form>
          </section>

          <section className="space-y-3 rounded-2xl border border-zinc-200 bg-white p-4 shadow-sm">
            <h2 className="text-sm font-semibold text-zinc-800">模拟收取</h2>
            <p className="text-xs text-zinc-500">
              也可用 POST /webhooks/mail/inbound JSON 推送。
            </p>
            <form
              className="space-y-2"
              onSubmit={inbound.onSubmit(mail_simulate_inbound, {
                onSuccess: () => setTip('已写入收件箱'),
              })}
            >
              <input
                className="w-full rounded-lg border border-zinc-300 px-3 py-2 text-sm"
                placeholder="发件人"
                value={inbound.data.from}
                onChange={(e) => inbound.setData('from', e.target.value)}
              />
              <input
                className="w-full rounded-lg border border-zinc-300 px-3 py-2 text-sm"
                placeholder="主题"
                value={inbound.data.subject}
                onChange={(e) => inbound.setData('subject', e.target.value)}
              />
              <textarea
                className="w-full rounded-lg border border-zinc-300 px-3 py-2 text-sm"
                rows={3}
                value={inbound.data.text}
                onChange={(e) => inbound.setData('text', e.target.value)}
              />
              <button
                type="submit"
                disabled={inbound.processing}
                className="rounded-lg bg-zinc-800 px-3 py-2 text-sm font-medium text-white disabled:opacity-50"
              >
                收取一封
              </button>
            </form>
          </section>

          <section className="space-y-3 rounded-2xl border border-zinc-200 bg-white p-4 shadow-sm lg:col-span-2">
            <h2 className="text-sm font-semibold text-zinc-800">手机验证码</h2>
            <form
              className="flex flex-wrap items-end gap-2"
              onSubmit={phoneForm.onSubmit(sms_send_code, {
                onSuccess: () =>
                  setTip('验证码已发送（开发环境看服务端日志 sms:otp）'),
              })}
            >
              <label className="min-w-[10rem] flex-1 space-y-1">
                <span className="text-xs text-zinc-500">手机号</span>
                <input
                  className="w-full rounded-lg border border-zinc-300 px-3 py-2 text-sm"
                  value={phoneForm.data.phone}
                  onChange={(e) => phoneForm.setData('phone', e.target.value)}
                />
              </label>
              <button
                type="submit"
                disabled={phoneForm.processing}
                className="rounded-lg bg-teal-700 px-3 py-2 text-sm font-medium text-white disabled:opacity-50"
              >
                发送验证码
              </button>
            </form>
            <form
              className="flex flex-wrap items-end gap-2"
              onSubmit={phoneForm.onSubmit(sms_verify_code, {
                onSuccess: () => setTip('手机验证通过'),
              })}
            >
              <label className="min-w-[8rem] space-y-1">
                <span className="text-xs text-zinc-500">验证码</span>
                <input
                  className="w-full rounded-lg border border-zinc-300 px-3 py-2 text-sm"
                  value={phoneForm.data.code}
                  onChange={(e) => phoneForm.setData('code', e.target.value)}
                />
              </label>
              <button
                type="submit"
                disabled={phoneForm.processing}
                className="rounded-lg border border-teal-700 px-3 py-2 text-sm font-medium text-teal-800 disabled:opacity-50"
              >
                校验
              </button>
            </form>
            {phoneForm.errors.code || phoneForm.errors.phone ? (
              <p className="text-sm text-red-600">
                {phoneForm.errors.code || phoneForm.errors.phone}
              </p>
            ) : null}
          </section>
        </div>

        <div className="mt-8 grid gap-6 lg:grid-cols-2">
          <MessageList title={`发件箱 (${outbox.length})`} items={outbox} kind="out" />
          <MessageList title={`收件箱 (${inbox.length})`} items={inbox} kind="in" />
        </div>

        <section className="mt-6 rounded-2xl border border-zinc-200 bg-white p-4 shadow-sm">
          <h2 className="mb-3 text-sm font-semibold text-zinc-800">
            短信记录 ({smsSent.length})
          </h2>
          {smsSent.length === 0 ? (
            <p className="text-sm text-zinc-400">暂无</p>
          ) : (
            <ul className="space-y-2 text-sm">
              {smsSent.map((s) => (
                <li key={s.id} className="rounded-lg bg-zinc-50 px-3 py-2">
                  <div className="text-xs text-zinc-400">→ {s.to}</div>
                  <div className="text-zinc-800">{s.body}</div>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </main>
  )
}

function MessageList({
  title,
  items,
  kind,
}: {
  title: string
  items: MailboxPage['outbox']
  kind: 'in' | 'out'
}) {
  return (
    <section className="rounded-2xl border border-zinc-200 bg-white p-4 shadow-sm">
      <h2 className="mb-3 text-sm font-semibold text-zinc-800">{title}</h2>
      {items.length === 0 ? (
        <p className="text-sm text-zinc-400">暂无</p>
      ) : (
        <ul className="space-y-2 text-sm">
          {items.map((m) => (
            <li key={m.id} className="rounded-lg bg-zinc-50 px-3 py-2">
              <div className="text-xs text-zinc-400">
                {kind === 'out' ? `→ ${m.to}` : `← ${m.from}`} · {m.subject}
              </div>
              <div className="text-zinc-800">{m.text || '(空正文)'}</div>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
