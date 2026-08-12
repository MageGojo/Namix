//! 页面首屏壳由 **Rust** 产出，**不依赖 Node**。
//!
//! 历史实现曾 spawn `node public/build/ssr/_ssr.js` 做 React SSR；
//! 生产机常无 Node，island 会回退 SPA 再打 `/__namix/props`，在反代多应用
//! 场景下极易串到别的 Namix 实例 → `props 404`。
//!
//! 现行约定：
//! - **island**：Rust 输出 HTML 壳 + **内联 props**（`#__namix_page`），浏览器 mount/hydrate
//! - **ssr**：Rust 输出 HTML 壳 + CSS；正文由 Rust 渲染器填充。渲染器尚未提供正文时，
//!   上层自动使用 island 壳 + 内联 props，让页面保持可用而不是返回空 `200`
//! - Vite 的 `public/build/ssr/` 产物不再被运行时调用（可忽略）

use serde_json::Value;

/// 可选的服务端正文 HTML。
///
/// 当前 Rust 壳没有组件级正文渲染器，因此返回空正文；调用方必须将其视为
/// “需要客户端挂载”，而不是可直接发送的纯 SSR 文档。
pub fn render_html(_component: &str, _props: &Value, _url: &str) -> Result<String, String> {
    Ok(String::new())
}
