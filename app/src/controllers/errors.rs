//! 可选 HTML 错误页（404 / 403 / 500 / 429 …）。
//!
//! 不注册则框架保持默认。注册方式见 `routes/web.rs` 的 `.error_pages(errors::page)`。

use crate::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct ErrorsPage {
    pub status: u16,
    pub title: String,
    pub message: String,
}

/// 所有 HTML 错误共用一页；也可按状态拆成 `not_found` / `forbidden` 再 `.error_page(404, …)`。
pub fn page(req: &Request, error: ErrorPage) -> Response {
    let (title, message) = copy(error.status, &error.message);
    req.view(Page::Errors)
        .ssr()
        .title(title)
        .data(ErrorsPage {
            status: error.status,
            title: title.to_string(),
            message,
        })
        .render()
}

fn copy(status: u16, message: &str) -> (&'static str, String) {
    let title = match status {
        401 => "请先登录",
        403 => "没有权限",
        404 => "页面不存在",
        409 => "无法完成操作",
        422 => "填写有误",
        429 => "请求过于频繁",
        500 => "出错了",
        _ => "无法完成请求",
    };
    let body = match (status, message) {
        (404, "not found") => "地址不存在，或已经换了地方。".into(),
        (403, "forbidden") => "你没有权限查看这一页。".into(),
        (401, "authentication required") => "登录后再访问。".into(),
        (429, "too many requests") => "请稍后再试。".into(),
        (500, "internal server error") => "服务暂时出了问题，请稍后再试。".into(),
        _ => message.to_string(),
    };
    (title, body)
}
