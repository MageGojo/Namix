//! 控制器视图能力 —— Laravel `return view('x')->with(...);`。

use serde::Serialize;

use crate::core::controller::Controller as ControllerCore;
use crate::core::request::Request;
use crate::core::response::Response;

use super::{Document, DocumentTemplateError, RenderMode, View, ViewPage};

/// 控制器 = 跳转/flash（[`ControllerCore`]）+ 渲染视图。
///
/// ```ignore
/// // Laravel: return view('login', compact('error'))->with('count', $n);
/// req.view("login")
///     .ssr()
///     .title("登录")
///     .with("error", err)
///     .with("registeredCount", n)
///     .render()
///
/// // 类型化：req.render(Login { ... })
/// ```
pub trait Controller: ControllerCore {
    /// 开始拼视图（链式 `with` / `title` / `ssr` → `render`）。
    fn view(&self, name: impl Into<String>) -> ViewBag<'_>;

    /// 类型化页面：`req.render(Login { .. })`。
    fn render<P: ViewPage>(&self, page: P) -> Response;
}

impl Controller for Request {
    fn view(&self, name: impl Into<String>) -> ViewBag<'_> {
        ViewBag {
            req: self,
            view: View::make(name),
        }
    }

    fn render<P: ViewPage>(&self, page: P) -> Response {
        page.render_page(self)
    }
}

/// Laravel 风格视图构建器：`req.view("login").with(...).render()`。
pub struct ViewBag<'a> {
    req: &'a Request,
    view: View,
}

impl<'a> ViewBag<'a> {
    /// `->with('key', $value)`
    pub fn with(mut self, key: impl AsRef<str>, value: impl Serialize) -> Self {
        self.view = self.view.prop(key.as_ref(), value);
        self
    }

    /// 一次合并多字段：`view('x', ['a' => 1, 'b' => 2])`
    pub fn data(mut self, props: impl Serialize) -> Self {
        self.view = self.view.data(props);
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.view = self.view.title(title);
        self
    }

    pub fn document(mut self, document: Document) -> Self {
        self.view = self.view.document(document);
        self
    }

    pub fn lang(mut self, lang: impl AsRef<str>) -> Self {
        self.view = self.view.lang(lang);
        self
    }

    pub fn html(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.view = self.view.html(name, value);
        self
    }

    pub fn html_attr(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.html(name, value)
    }

    pub fn html_class(mut self, class: impl AsRef<str>) -> Self {
        self.view = self.view.html_class(class);
        self
    }

    pub fn body(mut self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.view = self.view.body(name, value);
        self
    }

    pub fn body_attr(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.body(name, value)
    }

    pub fn body_class(mut self, class: impl AsRef<str>) -> Self {
        self.view = self.view.body_class(class);
        self
    }

    pub fn set_body_class(mut self, class: impl AsRef<str>) -> Self {
        self.view = self.view.set_body_class(class);
        self
    }

    /// 追加到文档 `<head>` 的可信 HTML。
    pub fn head(mut self, html: impl AsRef<str>) -> Self {
        self.view = self.view.head(html);
        self
    }

    pub fn template(mut self, html: impl Into<String>) -> Self {
        self.view = self.view.template(html);
        self
    }

    pub fn template_file(
        mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, DocumentTemplateError> {
        self.view = self.view.template_file(path)?;
        Ok(self)
    }

    pub fn mode(mut self, mode: RenderMode) -> Self {
        self.view = self.view.mode(mode);
        self
    }

    pub fn ssr(self) -> Self {
        self.mode(RenderMode::Ssr)
    }

    /// Pure native SSR with a trusted HTML body.
    pub fn ssr_html(mut self, body: impl Into<String>) -> Self {
        self.view = self.view.server_html(body);
        self
    }

    pub fn spa(self) -> Self {
        self.mode(RenderMode::Spa)
    }

    pub fn island(self) -> Self {
        self.mode(RenderMode::Island)
    }

    /// 渲染并消费 flash cookie。
    pub fn render(self) -> Response {
        self.view.render(self.req).consume_flash(self.req)
    }
}
