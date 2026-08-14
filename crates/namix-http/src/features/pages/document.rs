//! 文档壳：开发者拥有 `<html>` / `<body>` / `<head>`，不依赖 class。
//!
//! 暗亮色默认走 `data-theme` + `color-scheme` + CSS 变量；`class="dark"` 只是可选糖。

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::core::request::Request;

/// Cookie：`dark` | `light` | `system`（可读，供首包与客户端切换）。
pub const THEME_COOKIE: &str = "namix_theme";

/// 阻塞脚本：在 CSS 前按 cookie / `prefers-color-scheme` 写上 `data-theme`。
pub const THEME_SCRIPT: &str = concat!(
    "<script>(function(){var m=document.cookie.match(/(?:^|; )namix_theme=([^;]*)/);",
    "var t=m?decodeURIComponent(m[1]):\"system\";",
    "var d=t===\"dark\"||(t!==\"light\"&&window.matchMedia(\"(prefers-color-scheme: dark)\").matches);",
    "var e=document.documentElement;e.setAttribute(\"data-theme\",d?\"dark\":\"light\");",
    "e.style.colorScheme=d?\"dark\":\"light\";})();</script>"
);

/// 不靠页面 class 也能换底色：选择器挂在 `html[data-theme]` 上。
pub const THEME_STYLE: &str = concat!(
    "<style>html{color-scheme:light;background:#fafafa;color:#18181b}",
    "html[data-theme=dark]{color-scheme:dark;background:#09090b;color:#fafafa}",
    "body{min-height:100vh;background:inherit;color:inherit}</style>"
);

const DEFAULT_LANG: &str = "zh-CN";
const DEFAULT_BODY_CLASS: &[&str] = &["min-h-screen", "bg-zinc-50", "text-zinc-900", "antialiased"];

const DEFAULT_TEMPLATE: &str = "\
<!doctype html>\n\
<html{{html_attrs}}>\n\
<head>\n\
<meta charset=\"utf-8\"/>\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"/>\n\
{{extra_head}}<title>{{title}}</title>\n\
{{tags}}\n\
</head>\n\
<body{{body_attrs}}>\n\
{{app}}\n\
</body>\n\
</html>";

/// 从 `.html` 文件加载文档模板失败。
#[derive(Debug, Error)]
pub enum DocumentTemplateError {
    #[error("document template path is empty")]
    EmptyPath,
    #[error("document template path `{path}` must stay within the working directory")]
    Escape { path: String },
    #[error("document template `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// 一页 HTML 文档壳。空值表示「不覆盖」；与请求上的 [`Document`] 以及框架默认合并。
#[derive(Debug, Clone, Default)]
pub struct Document {
    lang: Option<String>,
    html_class: Vec<String>,
    html_attrs: Vec<(String, String)>,
    body_class: Vec<String>,
    replace_body_class: bool,
    body_attrs: Vec<(String, String)>,
    extra_head: String,
    template: Option<String>,
}

impl Document {
    pub fn new() -> Self {
        Self::default()
    }

    /// 框架默认壳（`lang=zh-CN` + 现有 body class）。内部合并的起点。
    pub fn shell_defaults() -> Self {
        Self {
            lang: Some(DEFAULT_LANG.into()),
            body_class: DEFAULT_BODY_CLASS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            ..Self::new()
        }
    }

    /// 读 `namix_theme` cookie，只写属性，不加 class。
    pub fn from_theme_cookie(req: &Request) -> Self {
        match req.cookie(THEME_COOKIE).map(str::trim) {
            Some("dark") => Self::new()
                .html("data-theme", "dark")
                .html("style", "color-scheme: dark"),
            Some("light") => Self::new()
                .html("data-theme", "light")
                .html("style", "color-scheme: light"),
            _ => Self::new(),
        }
    }

    /// 推荐：cookie 首包 `data-theme` + 阻塞脚本 + 文档级 CSS。页面不必写 `dark:` class。
    pub fn themed(req: &Request) -> Self {
        Self::from_theme_cookie(req)
            .head(THEME_SCRIPT)
            .head(THEME_STYLE)
            .set_body_class("")
    }

    pub fn lang(mut self, lang: impl AsRef<str>) -> Self {
        if let Some(lang) = sanitize_lang(lang.as_ref()) {
            self.lang = Some(lang);
        }
        self
    }

    /// `<html name="value">`。`class` / `lang` 会转到专用字段。
    pub fn html(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.push_attr(true, name.as_ref(), value.into())
    }

    pub fn html_attr(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.html(name, value)
    }

    pub fn html_class(mut self, class: impl AsRef<str>) -> Self {
        push_classes(&mut self.html_class, class.as_ref());
        self
    }

    /// `<body name="value">`。
    pub fn body(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.push_attr(false, name.as_ref(), value.into())
    }

    pub fn body_attr(self, name: impl AsRef<str>, value: impl Into<String>) -> Self {
        self.body(name, value)
    }

    pub fn body_class(mut self, class: impl AsRef<str>) -> Self {
        push_classes(&mut self.body_class, class.as_ref());
        self
    }

    /// 换掉框架默认 body class（而不是追加）。
    pub fn set_body_class(mut self, class: impl AsRef<str>) -> Self {
        self.body_class.clear();
        self.replace_body_class = true;
        push_classes(&mut self.body_class, class.as_ref());
        self
    }

    /// 追加到 `<head>` 的可信 HTML（favicon、`<style>`、`<meta>`、阻塞脚本）。
    /// 用户输入必须先转义；不要把未过滤的字符串拼进来。
    pub fn head(mut self, html: impl AsRef<str>) -> Self {
        append_head(&mut self.extra_head, html.as_ref());
        self
    }

    /// 整份文档模板。占位符：`{{html_attrs}}` `{{body_attrs}}` `{{title}}`
    /// `{{extra_head}}` `{{tags}}` `{{app}}`。不写 class 也可以，属性全走 `{{html_attrs}}`。
    pub fn template(mut self, html: impl Into<String>) -> Self {
        let html = html.into();
        self.template = (!html.trim().is_empty()).then_some(html);
        self
    }

    /// 从 HTML 文件加载整份文档模板（相对工作目录，或绝对路径）。
    ///
    /// 推荐放在 `src/views/layouts/app.html`。占位符与 [`Self::template`] 相同。
    /// 相对路径禁止 `..`，避免模板路径逃逸出工作目录。
    ///
    /// ```ignore
    /// Boot::new("main")
    ///     .document(Document::new().template_file("src/views/layouts/app.html")?)
    /// ```
    pub fn template_file(self, path: impl AsRef<Path>) -> Result<Self, DocumentTemplateError> {
        let path = path.as_ref();
        let resolved = resolve_template_path(path)?;
        let html =
            std::fs::read_to_string(&resolved).map_err(|source| DocumentTemplateError::Io {
                path: resolved.display().to_string(),
                source,
            })?;
        Ok(self.template(html))
    }

    pub fn merge(mut self, overlay: Document) -> Self {
        self.apply(&overlay);
        self
    }

    pub(crate) fn resolve(request: Option<&Document>, page: &Document) -> Self {
        let custom_template =
            page.template.is_some() || request.is_some_and(|document| document.template.is_some());
        let mut out = if custom_template {
            Self {
                lang: Some(DEFAULT_LANG.into()),
                ..Self::new()
            }
        } else {
            Self::shell_defaults()
        };
        if let Some(request) = request {
            out.apply(request);
        }
        out.apply(page);
        out
    }

    pub(crate) fn render_shell(&self, title: &str, tags: &str, app: &str) -> String {
        let tpl = self.template.as_deref().unwrap_or(DEFAULT_TEMPLATE);
        tpl.replace("{{html_attrs}}", &self.html_attrs_suffix())
            .replace("{{body_attrs}}", &self.body_attrs_suffix())
            .replace("{{extra_head}}", &self.extra_head)
            .replace("{{title}}", &html_escape(title))
            .replace("{{tags}}", tags)
            .replace("{{app}}", app)
    }

    fn apply(&mut self, overlay: &Document) {
        if overlay.lang.is_some() {
            self.lang.clone_from(&overlay.lang);
        }
        for token in &overlay.html_class {
            push_classes(&mut self.html_class, token);
        }
        for (name, value) in &overlay.html_attrs {
            upsert_attr(&mut self.html_attrs, name, value);
        }
        if overlay.replace_body_class {
            self.body_class.clone_from(&overlay.body_class);
        } else {
            for token in &overlay.body_class {
                push_classes(&mut self.body_class, token);
            }
        }
        for (name, value) in &overlay.body_attrs {
            upsert_attr(&mut self.body_attrs, name, value);
        }
        if !overlay.extra_head.is_empty() {
            append_head(&mut self.extra_head, overlay.extra_head.trim_end());
        }
        if overlay.template.is_some() {
            self.template.clone_from(&overlay.template);
        }
    }

    fn push_attr(mut self, html: bool, name: &str, value: String) -> Self {
        if name.eq_ignore_ascii_case("class") {
            return if html {
                self.html_class(value)
            } else {
                self.body_class(value)
            };
        }
        if html && name.eq_ignore_ascii_case("lang") {
            return self.lang(value);
        }
        if !is_safe_attr_name(name) {
            return self;
        }
        let attrs = if html {
            &mut self.html_attrs
        } else {
            &mut self.body_attrs
        };
        upsert_attr(attrs, name, &value);
        self
    }

    fn html_attrs_suffix(&self) -> String {
        let mut tag = String::new();
        let lang = self.lang.as_deref().unwrap_or(DEFAULT_LANG);
        tag.push_str(" lang=\"");
        tag.push_str(&html_escape_attr(lang));
        tag.push('"');
        append_class_and_attrs(&mut tag, &self.html_class, &self.html_attrs);
        tag
    }

    fn body_attrs_suffix(&self) -> String {
        let mut tag = String::new();
        append_class_and_attrs(&mut tag, &self.body_class, &self.body_attrs);
        tag
    }
}

fn append_class_and_attrs(tag: &mut String, class: &[String], attrs: &[(String, String)]) {
    if !class.is_empty() {
        tag.push_str(" class=\"");
        tag.push_str(&html_escape_attr(&class.join(" ")));
        tag.push('"');
    }
    for (name, value) in attrs {
        if !is_safe_attr_name(name) {
            continue;
        }
        tag.push(' ');
        tag.push_str(name);
        tag.push_str("=\"");
        tag.push_str(&html_escape_attr(value));
        tag.push('"');
    }
}

fn append_head(target: &mut String, html: &str) {
    let html = html.trim();
    if html.is_empty() {
        return;
    }
    if !target.is_empty() && !target.ends_with('\n') {
        target.push('\n');
    }
    target.push_str(html);
    if !target.ends_with('\n') {
        target.push('\n');
    }
}

fn push_classes(target: &mut Vec<String>, raw: &str) {
    for token in raw.split_whitespace() {
        if !token.is_empty() && !target.iter().any(|existing| existing == token) {
            target.push(token.to_string());
        }
    }
}

fn upsert_attr(attrs: &mut Vec<(String, String)>, name: &str, value: &str) {
    if let Some(existing) = attrs
        .iter_mut()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
    {
        existing.1 = value.to_string();
        return;
    }
    attrs.push((name.to_string(), value.to_string()));
}

fn is_safe_attr_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':')) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    !lower.starts_with("on") && lower != "srcdoc"
}

fn sanitize_lang(lang: &str) -> Option<String> {
    let lang = lang.trim();
    let mut parts = lang.split('-');
    let primary = parts.next()?;
    if !(2..=8).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = primary.to_string();
    for part in parts {
        if part.is_empty() || part.len() > 8 || !part.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        out.push('-');
        out.push_str(part);
    }
    Some(out)
}

fn resolve_template_path(path: &Path) -> Result<PathBuf, DocumentTemplateError> {
    if path.as_os_str().is_empty() {
        return Err(DocumentTemplateError::EmptyPath);
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(DocumentTemplateError::Escape {
            path: path.display().to_string(),
        });
    }
    Ok(std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path))
}

pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, Method, Uri};

    fn req_with_cookie(cookie: &str) -> Request {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", cookie.parse().unwrap());
        Request::new(
            Method::GET,
            Uri::from_static("/"),
            headers,
            bytes::Bytes::new(),
        )
    }

    #[test]
    fn theme_cookie_dark_sets_data_theme_not_class() {
        let doc = Document::resolve(
            Some(&Document::from_theme_cookie(&req_with_cookie(
                "namix_theme=dark",
            ))),
            &Document::new(),
        );
        let html = doc.render_shell("", "", "");
        assert!(html.contains("data-theme=\"dark\""));
        assert!(html.contains("color-scheme: dark"));
        assert!(!html.contains("class=\"dark\""));
        assert!(html.contains("<html lang=\"zh-CN\""));
    }

    #[test]
    fn template_can_omit_classes_entirely() {
        let doc = Document::resolve(
            None,
            &Document::new()
                .html("data-theme", "dark")
                .body("id", "root")
                .template(
                    "<!doctype html><html{{html_attrs}}><body{{body_attrs}}>{{app}}</body></html>",
                ),
        );
        let out = doc.render_shell("Hi", "", "<p>ok</p>");
        assert!(out.contains("data-theme=\"dark\""));
        assert!(out.contains("id=\"root\""));
        assert!(out.contains("<p>ok</p>"));
        assert!(!out.contains("class="));
    }

    #[test]
    fn unsafe_attr_names_are_dropped() {
        let doc = Document::new()
            .html("onclick", "alert(1)")
            .html("data-ok", "yes")
            .html("foo bar", "x");
        let tag = Document::resolve(None, &doc).render_shell("", "", "");
        assert!(!tag.contains("onclick"));
        assert!(!tag.contains("foo bar"));
        assert!(tag.contains("data-ok=\"yes\""));
    }

    #[test]
    fn attr_values_are_escaped() {
        let doc = Document::new().body("data-title", r#"a"b"#);
        let tag = Document::resolve(None, &doc).render_shell("", "", "");
        assert!(tag.contains("data-title=\"a&quot;b\""));
    }

    #[test]
    fn template_file_loads_html_from_disk() {
        let dir = std::env::temp_dir().join(format!(
            "namix-doc-tpl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.html");
        std::fs::write(
            &path,
            "<!doctype html><html{{html_attrs}}><body{{body_attrs}}>{{app}}</body></html>",
        )
        .unwrap();

        let doc = Document::new()
            .html("data-theme", "dark")
            .template_file(&path)
            .unwrap();
        let out = Document::resolve(None, &doc).render_shell("Hi", "", "<p>ok</p>");
        assert!(out.contains("data-theme=\"dark\""));
        assert!(out.contains("<p>ok</p>"));
        assert!(!out.contains("class="));

        let err = Document::new().template_file("../escape.html").unwrap_err();
        assert!(matches!(err, DocumentTemplateError::Escape { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
