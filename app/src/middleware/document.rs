//! 文档壳：按 cookie 合并暗亮色，避免首包闪白。

use namix::prelude::*;

pub async fn apply(mut req: Request, next: Next) -> Response {
    let base = req.get::<Document>().cloned().unwrap_or_default();
    req.set(base.merge(Document::themed(&req)));
    next.run(req).await
}
