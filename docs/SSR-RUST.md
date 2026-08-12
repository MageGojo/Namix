# SSR / Island · Rust 运行时（无 Node 依赖）

## 结论

**`ssr` / `island` 的服务端路径由 Rust 实现，运行时不 spawn Node。**

| 模式 | Rust 做什么 | 浏览器做什么 |
|---|---|---|
| `.ssr_html(html)` | 可信的 Rust 模板正文 + CSS | 不加载 React 入口 |
| `.ssr()` | 优先取 Rust 正文；正文为空时输出 Island 壳和内联 props | 纯正文时不运行；回退时 `createRoot` |
| `.island()` | HTML 壳、可选 SSR 正文 + **内联** `#__namix_page` props | 有正文时 `hydrateRoot`，否则 `createRoot` |
| `.spa()` | 只发 key，客户端 `GET /__namix/props/:key` | 适合单应用直连；**多应用反代易串台** |

`.ssr()` 的非空保障很重要：当前 Rust 运行时不会执行 TSX，因此没有配置原生正文时，框架自动切到内联 Island，而不是返回只有空 `#app` 的成功响应。要得到确定的纯 HTML，先用模板引擎生成经过转义的正文，再传给 `.ssr_html(...)`。

## 为什么改

旧实现：`spawn(node public/build/ssr/_ssr.js)`。  
生产机无 Node → island 失败 → 回退 SPA → 拉 `/__namix/props` → 在「一机多 Namix」（如咸鱼云控 + 咸鱼找图）下打到错误进程 → **props 404**。

## 代码

- `crates/namix-http/src/features/pages/ssr.rs` — Rust 壳，不再调用 Node  
- `mod.rs` — island **禁止**再回退 SPA props

## 应用侧建议

- 要交互的页面：`.island()`
- 纯 Rust 模板页面：`.ssr_html(rendered_html)`
- SSR 优先、允许客户端兜底的 React 页面：`.ssr()`
- 多应用共用反代时：**不要**依赖裸路径 `/__namix/props`（除非反代能按应用隔离）
- Vite `npm run build` 的 `public/build/ssr/` 可保留作将来实验，**运行时不读**
