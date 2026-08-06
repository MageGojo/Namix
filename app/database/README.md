# database/ — Toasty 迁移产物（单应用）

方向：**Model → 表 / 迁移 SQL**（不能反过来）。

| 概念 | 位置 |
|------|------|
| 实体 | `src/models/*.rs` |
| 碰库逻辑 | `src/services/*.rs` |
| SQLite 文件 | `storage/namix.db` |
| 迁移 SQL | `migrations/` |

```bash
cargo run -p app --bin seed
cargo run -p app --bin toasty -- migration generate
sqlite3 storage/namix.db ".tables"
```
