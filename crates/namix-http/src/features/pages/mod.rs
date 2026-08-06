//! Laravel 风格 View：`Home { .. }.render(&req)` → `app/src/views/home.tsx`。
//!
//! - Props：`#[derive(Serialize, ViewProps)]` 一次定义，自动生成 TS 类型
//! - 路由：不进页面载荷；TSX 用 Boot 生成的 `routes.ts`（`route.main.home()`）
//! - 模式：
//!   - `spa`（默认）：空 `#app` + props key → 客户端 `createRoot`
//!   - `ssr`：服务端出 HTML + CSS，无 JSON、不加载客户端 JS
//!   - `island`：服务端 HTML + 内联 props，客户端可交互（当前整页 hydrate；后续拆岛）

mod assets;
mod controller_view;
mod props_store;
mod ssr;

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::core::controller::{html, json_raw, with_status};
use crate::core::request::Request;
use crate::core::response::{IntoResponse, Response};
use crate::core::routing::{Route, Router};
use http::StatusCode;

pub use assets::asset_routes;
pub use controller_view::{Controller, ViewBag};

pub fn enabled() -> bool {
    true
}

/// 页面渲染模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// 空 `#app` + props key（默认）
    #[default]
    Spa,
    /// 纯 SSR：HTML + CSS，无 props JSON、不加载客户端 JS
    Ssr,
    /// Island：SSR HTML + 客户端可交互（当前整页 hydrate；后续按岛拆分）
    Island,
}

impl RenderMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "spa" | "client" | "csr" => Some(Self::Spa),
            // static/html 兼容旧写法 → 纯 SSR
            "ssr" | "server" | "static" | "ssg" | "html" => Some(Self::Ssr),
            "island" | "hydrate" => Some(Self::Island),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spa => "spa",
            Self::Ssr => "ssr",
            Self::Island => "island",
        }
    }
}

/// 页面 props 契约：由 `#[derive(ViewProps)]` 实现。
pub trait ViewPage: Serialize + Sized {
    const COMPONENT: &'static str;

    /// 默认 SPA；在 `#[view("x", mode = "ssr"|"island")]` 时由宏覆写。
    const RENDER_MODE: RenderMode = RenderMode::Spa;

    fn document_title(&self) -> Option<&str> {
        None
    }

    fn render_page(self, req: &Request) -> Response {
        // 读 flash 的页面一并清掉 cookie，不必在控制器里再写 consume_flash
        self.render_as(req, Self::RENDER_MODE).consume_flash(req)
    }

    /// 覆盖 `#[view]` 默认模式（同组件多模式演示时用；业务页优先写在属性上）。
    fn render_as(self, req: &Request, mode: RenderMode) -> Response {
        let title = self.document_title().map(str::to_owned);
        let mut view = View::make(Self::COMPONENT).mode(mode).with(self);
        if let Some(t) = title {
            view = view.title(t);
        }
        view.render(req)
    }
}

/// `async fn island(req) -> Island` — 返回页面类型即渲染。
impl<T: ViewPage> crate::core::response::Respond for T {
    fn respond(self, req: &Request) -> Response {
        self.render_page(req)
    }
}

/// 无请求时拼 View：`view("home").data(props)`（有 Request 时用 `req.view("home")`）。
pub fn view(component: impl Into<String>) -> View {
    View::make(component)
}

#[derive(Debug, Clone)]
pub struct View {
    component: String,
    props: Value,
    title: Option<String>,
    mode: RenderMode,
}

impl View {
    pub fn make(component: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            props: Value::Object(Map::new()),
            title: None,
            mode: RenderMode::Spa,
        }
    }

    /// 合并对象 props（同 Laravel 一次传入数组）。
    pub fn data(mut self, props: impl Serialize) -> Self {
        let raw = slim_value(serde_json::to_value(props).unwrap_or(Value::Null));
        self.props = merge_props(self.props, raw);
        self
    }

    /// 兼容旧名：等同 [`View::data`]。
    pub fn with(self, props: impl Serialize) -> Self {
        self.data(props)
    }

    /// 单字段：`->with('error', $msg)`。
    pub fn prop(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        let key = key.into();
        let val = slim_value(serde_json::to_value(value).unwrap_or(Value::Null));
        match &mut self.props {
            Value::Object(map) => {
                if !matches!(val, Value::Null) {
                    map.insert(key, val);
                }
            }
            other => {
                let mut map = Map::new();
                if !matches!(val, Value::Null) {
                    map.insert(key, val);
                }
                *other = Value::Object(map);
            }
        }
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// 运行时覆盖渲染模式（优先于 `#[view(..., mode = ...)]`）。
    pub fn mode(mut self, mode: RenderMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn render(self, req: &Request) -> Response {
        let url = req.uri().path().to_string();
        let payload = json!({
            "component": self.component,
            "props": self.props,
            "url": url,
        });

        if wants_props_json(req) {
            // 必须用 json_raw：json(String) 会再包一层引号，软导航会拿不到 component
            return json_raw(payload.to_string());
        }

        let title = self
            .title
            .clone()
            .unwrap_or_else(|| format!("Namix · {}", self.component));

        match self.mode {
            RenderMode::Ssr => match ssr::render_html(&self.component, &self.props, &url) {
                Ok(body_html) => html(document_shell_ssr(&title, &self.component, &body_html)),
                Err(err) => {
                    eprintln!("[namix pages] SSR failed ({err}); falling back to SPA");
                    let key = props_store::put(self.component.clone(), self.props, url);
                    html(document_shell_spa(&title, &self.component, &key))
                }
            },
            RenderMode::Island => match ssr::render_html(&self.component, &self.props, &url) {
                Ok(body_html) => html(document_shell_island(
                    &title,
                    &self.component,
                    &body_html,
                    &payload.to_string(),
                )),
                Err(err) => {
                    eprintln!("[namix pages] island SSR failed ({err}); falling back to SPA");
                    let key = props_store::put(self.component.clone(), self.props, url);
                    html(document_shell_spa(&title, &self.component, &key))
                }
            },
            RenderMode::Spa => {
                let key = props_store::put(self.component.clone(), self.props, url);
                html(document_shell_spa(&title, &self.component, &key))
            }
        }
    }
}

impl IntoResponse for View {
    fn into_response(self) -> Response {
        let key = props_store::put(self.component.clone(), self.props, "/".into());
        let title = self
            .title
            .unwrap_or_else(|| format!("Namix · {}", self.component));
        html(document_shell_spa(&title, &self.component, &key))
    }
}

fn merge_props(base: Value, extra: Value) -> Value {
    match (base, extra) {
        (Value::Object(mut a), Value::Object(b)) => {
            for (k, v) in b {
                a.insert(k, v);
            }
            Value::Object(a)
        }
        (_, extra) => extra,
    }
}

fn slim_value(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, child) in map {
                let child = slim_value(child);
                match &child {
                    // 空数组要保留：前端常写 items.length / items.map，去掉会白屏
                    Value::Null => continue,
                    Value::String(s) if s.is_empty() => continue,
                    Value::Object(o) if o.is_empty() => continue,
                    _ => {
                        out.insert(k, child);
                    }
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(slim_value)
                .filter(|x| !matches!(x, Value::Null))
                .collect(),
        ),
        other => other,
    }
}

fn wants_props_json(req: &Request) -> bool {
    matches!(
        req.header("x-namix-props"),
        Some("1") | Some("true") | Some("yes")
    ) || req
        .header("accept")
        .is_some_and(|a| a.contains("application/vnd.namix.props+json"))
}

fn document_shell_spa(title: &str, component: &str, key: &str) -> String {
    let title = html_escape(title);
    let component = html_escape_attr(component);
    let key = html_escape_attr(key);
    let tags = assets::script_tags();

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>{title}</title>
{tags}
</head>
<body class="min-h-screen bg-zinc-50 text-zinc-900 antialiased">
<div id="app" data-namix-view="{component}" data-namix-mode="spa" data-namix-key="{key}"></div>
</body>
</html>"#
    )
}

/// 纯 SSR：HTML + CSS，无 JSON、无客户端 JS。
fn document_shell_ssr(title: &str, component: &str, body_html: &str) -> String {
    let title = html_escape(title);
    let component = html_escape_attr(component);
    let tags = assets::css_tags();

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>{title}</title>
{tags}
</head>
<body class="min-h-screen bg-zinc-50 text-zinc-900 antialiased">
<div id="app" data-namix-view="{component}" data-namix-mode="ssr">{body_html}</div>
</body>
</html>"#
    )
}

/// Island：SSR HTML + 内联 props + 客户端 JS（可交互）。
fn document_shell_island(
    title: &str,
    component: &str,
    body_html: &str,
    props_json: &str,
) -> String {
    let title = html_escape(title);
    let component = html_escape_attr(component);
    let tags = assets::script_tags();
    let props_json = props_json.replace('<', "\\u003c");

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>{title}</title>
{tags}
</head>
<body class="min-h-screen bg-zinc-50 text-zinc-900 antialiased">
<div id="app" data-namix-view="{component}" data-namix-mode="island">{body_html}</div>
<script type="application/json" id="__namix_page">{props_json}</script>
</body>
</html>"#
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

async fn serve_props(req: Request) -> Response {
    let key = req.param("key").unwrap_or("").to_string();
    match props_store::take(&key) {
        Some(entry) => json_raw(
            json!({
                "component": entry.component,
                "props": entry.props,
                "url": entry.url,
            })
            .to_string(),
        ),
        None => with_status(StatusCode::NOT_FOUND, "props expired or unknown key"),
    }
}

pub fn routes() -> Router {
    asset_routes().merge(
        Route::get("/__namix/props/:key", serve_props)
            .name("__namix.props")
            .register(),
    )
}
