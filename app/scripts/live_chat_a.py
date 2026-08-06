#!/usr/bin/env python3
"""Connect as chat_a and keep a live WS session for manual testing."""

from __future__ import annotations

import asyncio
import http.cookiejar
import json
import pathlib
import re
import time
import urllib.request

import websockets

BASE = "http://127.0.0.1:3088"
APP = pathlib.Path(__file__).resolve().parents[1]
LOGIN_TS = (APP / "src/views/generated/actions/login.ts").read_text()
LOGIN_TOK = re.search(r"callRust\('([^']+)'", LOGIN_TS).group(1)
LOG = pathlib.Path("/tmp/namix_chat_live.log")
SENDQ = pathlib.Path("/tmp/namix_chat_a_sendq.txt")
SID_FILE = pathlib.Path("/tmp/namix_chat_a.sid")


def log(line: str) -> None:
    print(line, flush=True)
    with LOG.open("a", encoding="utf-8") as f:
        f.write(line + "\n")


def login() -> str:
    jar = http.cookiejar.CookieJar()
    payload = json.dumps(
        {
            "t": LOGIN_TOK,
            "i": {
                "username": "chat_a",
                "password": "ChatA12!",
                "redirect": "/chat",
            },
            "ts": int(time.time()),
        }
    ).encode()
    req = urllib.request.Request(
        BASE + "/api/a",
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    with opener.open(req, timeout=10) as resp:
        log(f"login {resp.status} {resp.read().decode()[:120]}")
    sid = next(c.value for c in jar if c.name == "namix_session")
    SID_FILE.write_text(sid)
    return sid


async def main() -> None:
    LOG.write_text("")
    SENDQ.write_text("")
    sid = login()
    uri = "ws://127.0.0.1:3088/ws/chat"
    async with websockets.connect(
        uri, additional_headers={"Cookie": f"namix_session={sid}"}
    ) as ws:
        log("[me] connected as chat_a")
        await asyncio.sleep(0.4)
        for m in (
            "嗨，我是 chat_a（AI 这边）",
            "你用 chat_b 登录后打开 /chat，回我一句就行",
            "我在线，随时聊～",
        ):
            await ws.send(json.dumps({"type": "chat", "text": m}))
            log(f"[me] {m}")
            await asyncio.sleep(0.7)

        last_pos = 0
        n = 0
        deadline = time.time() + 1800
        while time.time() < deadline:
            try:
                q = SENDQ.read_text(encoding="utf-8")
                if len(q) > last_pos:
                    chunk = q[last_pos:]
                    last_pos = len(q)
                    for line in chunk.splitlines():
                        line = line.strip()
                        if line:
                            await ws.send(json.dumps({"type": "chat", "text": line}))
                            log(f"[me] {line}")
            except Exception as e:
                log(f"[err] sendq {e}")

            try:
                raw = await asyncio.wait_for(ws.recv(), timeout=0.4)
                msg = json.loads(raw)
                kind = msg.get("type")
                if kind == "hello":
                    me = msg.get("me") or {}
                    log(f"[hello] id={me.get('id')} user={me.get('username')}")
                elif kind == "chat" and msg.get("username") != "chat_a":
                    who = msg.get("username")
                    uid = msg.get("userId")
                    text = (msg.get("text") or "").strip()
                    log(f"[them:{who}#{uid}] {text}")
                    n += 1
                    reply = reply_to(text, n)
                    await asyncio.sleep(0.25)
                    await ws.send(json.dumps({"type": "chat", "text": reply}))
                    log(f"[me] {reply}")
                elif kind == "system":
                    log(f"[sys] {msg.get('text')}")
                elif kind == "presence":
                    users = ", ".join(
                        f"{u.get('username')}#{u.get('id')}"
                        for u in (msg.get("users") or [])
                        if isinstance(u, dict)
                    )
                    log(f"[online] {users}")
            except asyncio.TimeoutError:
                pass
            except Exception as e:
                log(f"[err] {e}")
                break
        log("[me] session ended")


def reply_to(text: str, n: int) -> str:
    low = text.lower()
    if any(k in text for k in ("你好", "嗨", "hi", "hello")):
        return f"你好呀～我在线。你说：「{text[:60]}」"
    if any(k in text for k in ("延迟", "卡", "慢", "流畅")):
        return "感觉还行的话就说明 WS 推送正常；你那边卡不卡？"
    if any(k in text for k in ("再见", "拜拜", "结束", "bye")):
        return "好，先聊到这，有需要再喊我。"
    if text.isdigit():
        return f"收到数字 {text}，这是第 {n} 句回你。"
    snippet = text if len(text) <= 40 else text[:40] + "…"
    return f"收到（#{n}）：{snippet}"


if __name__ == "__main__":
    asyncio.run(main())
