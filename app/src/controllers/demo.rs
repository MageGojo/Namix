//! 渲染模式演示：`req.view(...).ssr() / .island()`。

use namix::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct DemoItem {
    pub id: u32,
    pub title: String,
    pub summary: String,
}

/// 页面 props 契约 → `views/generated/DemoPage.ts`（TSX 禁止手写重复 Props）
#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct DemoPage {
    pub title: String,
    pub page: u32,
    pub per_page: u32,
    pub total: u32,
    pub total_pages: u32,
    pub items: Vec<DemoItem>,
}

const TOTAL: u32 = 100;
const PER_PAGE: u32 = 20;

fn page_num(req: &Request) -> u32 {
    req.query_or("page", "1")
        .parse::<u32>()
        .unwrap_or(1)
        .max(1)
}

fn page_data(page: u32, title: impl Into<String>) -> DemoPage {
    let total = TOTAL;
    let per_page = PER_PAGE;
    let total_pages = total.div_ceil(per_page).max(1);
    let page = page.clamp(1, total_pages);
    let start = (page - 1) * per_page;
    let end = (start + per_page).min(total);

    let items = (start + 1..=end)
        .map(|id| DemoItem {
            id,
            title: format!("条目 #{id:03}"),
            summary: format!("这是第 {id} 条演示数据，纯 SSR 渲染，无客户端 hydrate。"),
        })
        .collect();

    DemoPage {
        title: title.into(),
        page,
        per_page,
        total,
        total_pages,
        items,
    }
}

pub async fn ssr(req: Request) -> Response {
    let data = page_data(page_num(&req), "纯渲染分页演示");
    req.view("demo")
        .ssr()
        .title(data.title.clone())
        .data(data)
        .render()
}

pub async fn island(req: Request) -> Response {
    let mut data = page_data(page_num(&req), "Island 分页演示");
    data.items = data
        .items
        .into_iter()
        .map(|item| DemoItem {
            summary: format!(
                "第 {} 条 · Island：SSR HTML + 内联 props + 客户端 hydrate。",
                item.id
            ),
            ..item
        })
        .collect();

    req.view("island")
        .island()
        .title(data.title.clone())
        .data(data)
        .render()
}
