# 技术决策

## 2026-08-05：路由采用宏 DSL，保留链式 API

`Route`/`Router` 链式 API 保留给动态构造和底层场景；业务路由默认使用 `routes!`。它将大量 `.merge(Route::… .register())` 压缩为可分组的声明式表，同时继续产出同一份命名路由 catalog。DSL 现覆盖 HTTP、PATCH 和 WebSocket；WebSocket 不接受 HTTP 中间件，以避免产生看似生效但实际不进入 Upgrade 流程的配置。

## 2026-08-05：跳转目标只使用站内绝对路径

`redirect` 参数、`previous_url` 和表单校验共享 `is_local_path`，拒绝完整 URL、协议相对 URL、反斜杠变体与控制字符。业务表单用 `Rule::LocalPath` 与 `Validated::local_path_or` 表达此要求。

## 2026-08-05：密码写入使用 Argon2id，保留旧演示数据兼容

新注册密码使用带随机盐的 Argon2id PHC 字符串。旧示例数据库内的 SHA-256 固定盐格式仅用于验证兼容，不再用于写入；下一阶段在成功登录后自动重哈希。

## 2026-08-05：全特性 Clippy 作为合并门槛

工作区采用 `cargo clippy --workspace --all-targets --all-features -- -D warnings`。对于 HTTP 层直接以 `Response` 作为错误值的少数 API，保留局部 `result_large_err` 说明，因为避免二次错误转换比装箱更符合公开框架 API 的使用体验。

## 2026-08-05：监听器错误不能被当作正常关机

`Server::run` 等待 Ctrl-C、终止信号或任一 listener 完成。此前第一个 listener 的 I/O 错误会在等待阶段被丢弃，随后显示为普通退出。现在该错误会向应用上浮，同时通知并等待其余 listener 停止，从而让端口冲突、TLS/QUIC 绑定失败等问题可诊断。
