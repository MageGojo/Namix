# Agent 开发约定（Namix）

Namix 是 **Rust + React 一体全栈**，不是前后端分离。

1. **必读 skill**：开始写业务或改 `app/` / `crates/namix*` 前，先读 [`.cursor/skills/namix/SKILL.md`](.cursor/skills/namix/SKILL.md)（个人机也有 `~/.cursor/skills/namix`）。
2. **文档入口**：[`docs/README.md`](docs/README.md)。
3. **默认路径**：控制器 `req.view` + `views/pages`；写操作用 `#[server]` 或经典 POST + `<CsrfField />`；契约以 Rust 生成的 `views/generated/*` 为准。
4. **禁止**：另起独立 SPA/API 仓库式结构、手改 `generated/`、props 下发授权字段、信表单 `user_id` 授权。
