//! 访问日志：方法 / 路径 / 状态 / 耗时（全局只挂这一个，不再另挂 timing）。

use std::time::Instant;

use namix::prelude::*;

pub async fn access_log(req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let path = req.path().to_string();
    let started = Instant::now();

    let response = next.run(req).await;

    let status = response.status().as_u16();
    let ms = started.elapsed().as_millis();
    let m = log::color_method(&method);
    let s = log::color_status(status);
    log::info!("{m} {path} → {s} ({ms}ms)");

    response
}

/// 兼容旧名。
pub async fn logger(req: Request, next: Next) -> Response {
    access_log(req, next).await
}
