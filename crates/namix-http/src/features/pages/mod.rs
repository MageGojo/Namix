//! Laravel 风格 View：`Home { .. }.render(&req)` → `app/src/views/home.tsx`。
//!
//! - Props：`#[derive(Serialize, ViewProps)]` 一次定义，自动生成 TS 类型
//! - 路由：不进页面载荷；TSX 用 Boot 生成的 `routes.ts`（`route.main.home()`）
//! - 模式：
//!   - `spa`（默认）：空 `#app` + props key → 客户端 `createRoot`
//!   - `ssr`：有 Rust 正文渲染器时输出纯 HTML；正文缺失时自动降级为内联客户端挂载
//!   - `island`：服务端 HTML + 内联 props，客户端可交互（当前整页 hydrate；后续拆岛）

mod assets;
mod controller_view;
mod document;
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
pub use document::{Document, DocumentTemplateError, THEME_COOKIE, THEME_SCRIPT};

pub fn enabled() -> bool {
    true
}

/// 页面渲染模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderMode {
    /// 空 `#app` + props key（默认）
    #[default]
    Spa,
    /// SSR 优先；正文渲染器缺失时使用内联 props 客户端挂载，绝不返回空页面
    Ssr,
    /// Island：SSR HTML + 客户端可交互（当前整页 hydrate；后续按岛拆分）
    Island,
}

impl RenderMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "spa" | "client" | "csr" => Some(Self::Spa),
            // static/html 兼容旧写法 → SSR 优先
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
    server_html: Option<String>,
    document: Document,
}

impl View {
    pub fn make(component: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            props: Value::Object(Map::new()),
            title: None,
            mode: RenderMode::Spa,
            server_html: None,
            document: Document::new(),
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

    pub fn document(mut self, document: Document) -> Self {
        self.document = self.document.merge(document);
        self
    }

    pub fn lang(self, lang: impl AsRef<str>) -> Self {
        self.document(Document::new().lang(lang))
    }

    pub fn html(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.document(Document::new().html(name, value))
    }

    pub fn html_attr(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.html(name, value)
    }

    pub fn html_class(self, class: impl AsRef<str>) -> Self {
        self.document(Document::new().html_class(class))
    }

    pub fn body(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.document(Document::new().body(name, value))
    }

    pub fn body_attr(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.body(name, value)
    }

    pub fn body_class(self, class: impl AsRef<str>) -> Self {
        self.document(Document::new().body_class(class))
    }

    pub fn set_body_class(self, class: impl AsRef<str>) -> Self {
        self.document(Document::new().set_body_class(class))
    }

    pub fn head(self, html: impl AsRef<str>) -> Self {
        self.document(Document::new().head(html))
    }

    pub fn template(self, html: impl Into<String>) -> Self {
        self.document(Document::new().template(html))
    }

    pub fn template_file(
        self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, DocumentTemplateError> {
        Ok(self.document(Document::new().template_file(path)?))
    }

    /// 运行时覆盖渲染模式（优先于 `#[view(..., mode = ...)]`）。
    pub fn mode(mut self, mode: RenderMode) -> Self {
        self.mode = mode;
        self
    }

    /// Supply trusted HTML rendered by a native Rust template and select pure
    /// SSR mode. User-controlled values must be escaped by the template.
    pub fn server_html(mut self, body: impl Into<String>) -> Self {
        self.server_html = Some(body.into());
        self.mode = RenderMode::Ssr;
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
            return page_response(json_raw(payload.to_string()));
        }

        let title = self
            .title
            .clone()
            .unwrap_or_else(|| format!("Namix · {}", self.component));
        let document = Document::resolve(req.get::<Document>(), &self.document);

        page_response(match self.mode {
            // SSR / Island 壳与 props 一律由 Rust 产出（见 ssr.rs），运行时不依赖 Node。
            RenderMode::Ssr => match self
                .server_html
                .map(Ok)
                .unwrap_or_else(|| ssr::render_html(&self.component, &self.props, &url))
            {
                Ok(body_html) if !body_html.trim().is_empty() => html(document_shell_ssr(
                    &title,
                    &self.component,
                    &body_html,
                    &document,
                )),
                Ok(_) => html(document_shell_island(
                    &title,
                    &self.component,
                    "",
                    &payload.to_string(),
                    &document,
                )),
                Err(error) => {
                    tracing::warn!(
                        component = %self.component,
                        error = %error,
                        "SSR renderer failed; using inline client rendering"
                    );
                    html(document_shell_island(
                        &title,
                        &self.component,
                        "",
                        &payload.to_string(),
                        &document,
                    ))
                }
            },
            RenderMode::Island => {
                let body_html =
                    ssr::render_html(&self.component, &self.props, &url).unwrap_or_default();
                // 内联 props，客户端 createRoot/hydrate；不再回退 SPA+/__namix/props
                html(document_shell_island(
                    &title,
                    &self.component,
                    &body_html,
                    &payload.to_string(),
                    &document,
                ))
            }
            RenderMode::Spa => {
                let key = props_store::put(self.component.clone(), self.props, url);
                html(document_shell_spa(&title, &self.component, &key, &document))
            }
        })
    }
}

impl IntoResponse for View {
    fn into_response(self) -> Response {
        let document = Document::resolve(None, &self.document);
        if let Some(body_html) = self.server_html {
            let title = self
                .title
                .unwrap_or_else(|| format!("Namix · {}", self.component));
            return page_response(html(document_shell_ssr(
                &title,
                &self.component,
                &body_html,
                &document,
            )));
        }
        let key = props_store::put(self.component.clone(), self.props, "/".into());
        let title = self
            .title
            .unwrap_or_else(|| format!("Namix · {}", self.component));
        page_response(html(document_shell_spa(
            &title,
            &self.component,
            &key,
            &document,
        )))
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

fn page_response(response: Response) -> Response {
    response
        .with_header("cache-control", "private, no-store")
        .with_header("vary", "accept, x-namix-props")
}

fn document_shell_spa(title: &str, component: &str, key: &str, document: &Document) -> String {
    let component = document::html_escape_attr(component);
    let key = document::html_escape_attr(key);
    render_document(
        document,
        title,
        &assets::script_tags(),
        &format!(
            r#"<div id="app" data-namix-view="{component}" data-namix-mode="spa" data-namix-key="{key}"></div>"#
        ),
    )
}

/// 纯 SSR：HTML + CSS，无 JSON、无客户端 JS。仅在正文非空时使用。
fn document_shell_ssr(
    title: &str,
    component: &str,
    body_html: &str,
    document: &Document,
) -> String {
    let component = document::html_escape_attr(component);
    render_document(
        document,
        title,
        &assets::css_tags(),
        &format!(
            r#"<div id="app" data-namix-view="{component}" data-namix-mode="ssr">{body_html}</div>"#
        ),
    )
}

/// Island：SSR HTML + 内联 props + 客户端 JS（可交互）。
fn document_shell_island(
    title: &str,
    component: &str,
    body_html: &str,
    props_json: &str,
    document: &Document,
) -> String {
    let component = document::html_escape_attr(component);
    let props_json = props_json.replace('<', "\\u003c");
    render_document(
        document,
        title,
        &assets::script_tags(),
        &format!(
            r#"<div id="app" data-namix-view="{component}" data-namix-mode="island">{body_html}</div>
<script type="application/json" id="__namix_page">{props_json}</script>"#
        ),
    )
}

fn render_document(document: &Document, title: &str, tags: &str, body_inner: &str) -> String {
    document.render_shell(title, tags, body_inner)
}

async fn serve_props(req: Request) -> Response {
    let key = req.param("key").unwrap_or("").to_string();
    match props_store::take(&key) {
        Some(entry) => page_response(json_raw(
            json!({
                "component": entry.component,
                "props": entry.props,
                "url": entry.url,
            })
            .to_string(),
        )),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::test_client::TestClient;

    async fn ssr_without_renderer(req: Request) -> Response {
        View::make("home")
            .mode(RenderMode::Ssr)
            .data(json!({"title": "Hello"}))
            .render(&req)
    }

    async fn island_with_hostile_json(req: Request) -> Response {
        View::make("home")
            .mode(RenderMode::Island)
            .data(json!({"text": "</script><script>alert(1)</script>"}))
            .render(&req)
    }

    async fn native_ssr(req: Request) -> Response {
        View::make("status")
            .title("Status")
            .server_html("<main><h1>Ready</h1></main>")
            .render(&req)
    }

    async fn document_overrides(req: Request) -> Response {
        req.view("home")
            .lang("en")
            .html("data-theme", "dark")
            .html("style", "color-scheme: dark")
            .body("data-page", "home")
            .body_class("marketing")
            .head("<meta name=\"theme-color\" content=\"#09090b\">")
            .title("Night")
            .ssr_html("<p>ok</p>")
            .render()
    }

    async fn document_owned_template(req: Request) -> Response {
        req.view("home")
            .template(
                "<!doctype html><html{{html_attrs}}><head><title>{{title}}</title>{{extra_head}}</head><body{{body_attrs}}>{{app}}</body></html>",
            )
            .html("data-theme", "dark")
            .body("id", "root")
            .title("Owned")
            .ssr_html("<p>ok</p>")
            .render()
    }

    async fn themed_from_request(mut req: Request) -> Response {
        req.set(Document::themed(&req));
        req.view("home")
            .title("Theme")
            .ssr_html("<p>ok</p>")
            .render()
    }

    #[tokio::test]
    async fn ssr_without_native_body_falls_back_to_inline_client_rendering() {
        let router = Route::get("/", ssr_without_renderer).register();
        let mut client = TestClient::new(router);
        let response = client.get("/").await;
        let body = response.text();

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(
            response.headers.get("cache-control").unwrap(),
            "private, no-store"
        );
        assert!(body.contains("data-namix-mode=\"island\""));
        assert!(body.contains("id=\"__namix_page\""));
        assert!(body.contains("\"component\":\"home\""));
        assert!(!body.contains("data-namix-mode=\"ssr\"></div>"));
    }

    #[tokio::test]
    async fn inline_page_payload_cannot_close_its_script_element() {
        let router = Route::get("/", island_with_hostile_json).register();
        let mut client = TestClient::new(router);
        let response = client.get("/").await;
        let body = response.text();

        assert!(!body.contains("</script><script>alert(1)</script>"));
        assert!(body.contains("\\u003c/script>\\u003cscript>alert(1)\\u003c/script>"));
    }

    #[tokio::test]
    async fn native_server_html_stays_pure_ssr() {
        let router = Route::get("/", native_ssr).register();
        let mut client = TestClient::new(router);
        let response = client.get("/").await;
        let body = response.text();

        assert!(body.contains("data-namix-mode=\"ssr\""));
        assert!(body.contains("<main><h1>Ready</h1></main>"));
        assert!(!body.contains("__namix_page"));
    }

    #[tokio::test]
    async fn document_shell_accepts_html_and_body_attributes() {
        let router = Route::get("/", document_overrides).register();
        let mut client = TestClient::new(router);
        let body = client.get("/").await.text().to_string();

        assert!(
            body.contains("<html lang=\"en\" data-theme=\"dark\" style=\"color-scheme: dark\">")
        );
        assert!(
            body.contains("class=\"min-h-screen bg-zinc-50 text-zinc-900 antialiased marketing\"")
        );
        assert!(body.contains("data-page=\"home\""));
        assert!(body.contains("<meta name=\"theme-color\" content=\"#09090b\">"));
        assert!(body.contains("<title>Night</title>"));
        assert!(!body.contains("class=\"dark\""));
    }

    #[tokio::test]
    async fn developer_template_does_not_require_classes() {
        let router = Route::get("/", document_owned_template).register();
        let mut client = TestClient::new(router);
        let body = client.get("/").await.text().to_string();

        assert!(body.contains("<html lang=\"zh-CN\" data-theme=\"dark\">"));
        assert!(body.contains("id=\"root\""));
        assert!(body.contains("<title>Owned</title>"));
        assert!(body.contains("<p>ok</p>"));
        assert!(!body.contains("class="));
    }

    #[tokio::test]
    async fn request_theme_cookie_sets_data_theme_without_class() {
        let router = Route::get("/", themed_from_request).register();
        let mut client = TestClient::new(router);
        client.set_cookie("namix_theme", "dark").unwrap();
        let body = client.get("/").await.text().to_string();

        assert!(body.contains("data-theme=\"dark\""));
        assert!(body.contains("color-scheme: dark"));
        assert!(body.contains("namix_theme"));
        assert!(body.contains("html[data-theme=dark]"));
        assert!(!body.contains("class=\"dark\""));
    }
}
