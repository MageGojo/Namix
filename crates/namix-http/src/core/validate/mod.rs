//! Laravel 风格验证器：字段用 enum，基础规则 + 自定义闭包。
//!
//! 业务更推荐 [`FormRequest`]：校验失败自动回表单页 + flash，控制器只拿结构体。

mod form;
mod presence;
mod rules;

use std::collections::HashMap;
use std::sync::RwLock;

use http::StatusCode;

pub use form::{FormRedirect, FormRequest, validator as form_validator};
pub use presence::{
    PresenceVerifier, clear_presence_verifier, install_presence_verifier, presence_exists,
};
pub use rules::Rule;

use crate::core::request::Request;
use crate::core::response::{IntoResponse, Response};
use crate::core::routing::NamedRoute;
use crate::core::upload::UploadedFile;

type ErrorTranslator = fn(&str) -> String;
static ERROR_TRANSLATOR: RwLock<Option<ErrorTranslator>> = RwLock::new(None);

/// Install `namix::trans_error` so flash / HTML 校验失败显示文案而不是码。
pub fn install_error_translator(translator: fn(&str) -> String) {
    *ERROR_TRANSLATOR.write().expect("error translator lock") = Some(translator);
}

pub fn clear_error_translator() {
    *ERROR_TRANSLATOR.write().expect("error translator lock") = None;
}

/// Look up a stable validation code. Without a translator, the code is returned as-is.
pub fn translate_error(code: &str) -> String {
    ERROR_TRANSLATOR
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().map(|translator| translator(code)))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| code.to_string())
}

/// 验证字段：用 enum 获得补全提示。
///
/// ```ignore
/// #[derive(FormField)]
/// enum LoginForm {
///     #[field = "email"]
///     Email,
///     #[field = "password"]
///     Password,
/// }
/// ```
pub trait Field: Copy + Send + Sync + 'static {
    fn name(self) -> &'static str;
}

type CustomRule<'a> =
    Box<dyn Fn(&str, &HashMap<String, String>) -> Result<(), String> + Send + Sync + 'a>;

/// 验证通过后的取值袋。
#[derive(Debug, Clone)]
pub struct Validated {
    values: HashMap<String, String>,
    files: HashMap<String, UploadedFile>,
}

impl Validated {
    pub fn get<F: Field>(&self, field: F) -> &str {
        self.values
            .get(field.name())
            .map(String::as_str)
            .unwrap_or("")
    }

    pub fn get_or<'a, F: Field>(&'a self, field: F, default: &'a str) -> &'a str {
        let v = self.get(field);
        if v.is_empty() { default } else { v }
    }

    pub fn raw(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn all(&self) -> &HashMap<String, String> {
        &self.values
    }

    pub fn file(&self, key: &str) -> Option<&UploadedFile> {
        self.files.get(key)
    }

    pub fn file_field<F: Field>(&self, field: F) -> Option<&UploadedFile> {
        self.file(field.name())
    }

    pub fn files(&self) -> &HashMap<String, UploadedFile> {
        &self.files
    }

    /// 读取站内跳转路径；外部 URL、`//host` 等返回默认值。
    pub fn local_path_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.raw(key)
            .filter(|value| crate::core::request::is_local_path(value))
            .unwrap_or(default)
    }
}

/// 验证失败。
#[derive(Debug, Clone)]
pub struct ValidationError {
    errors: HashMap<String, Vec<String>>,
}

impl ValidationError {
    pub fn errors(&self) -> &HashMap<String, Vec<String>> {
        &self.errors
    }

    pub fn first(&self) -> Option<&str> {
        self.errors.values().flatten().next().map(String::as_str)
    }

    pub fn message(&self) -> String {
        self.errors
            .iter()
            .flat_map(|(field, msgs)| msgs.iter().map(move |m| format!("{field}: {m}")))
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn redirect(self, to: impl AsRef<str>) -> Response {
        let msg = self
            .first()
            .map(translate_error)
            .unwrap_or_else(|| translate_error("validation.failed"));
        Response::redirect_see_other(to.as_ref()).with_flash_error(msg)
    }

    pub fn redirect_route(
        self,
        req: &Request,
        name: impl crate::core::routing::IntoRouteName,
    ) -> Response {
        match req.url(name) {
            Some(url) => self.redirect(url),
            None => self.into_response(),
        }
    }

    /// 退回上一页（`redirect` 查询参数 / Referer），并附带 error flash。
    pub fn redirect_back(self, req: &Request) -> Response {
        let back = req.previous_url().unwrap_or_else(|| "/".into());
        self.redirect(back)
    }

    pub fn redirect_to<R: NamedRoute>(self, req: &Request, route: R) -> Response {
        let msg = self
            .first()
            .map(translate_error)
            .unwrap_or_else(|| translate_error("validation.failed"));
        use crate::core::controller::Controller;
        req.redirect_error_to(route, msg)
    }
}

impl IntoResponse for ValidationError {
    fn into_response(self) -> Response {
        Response::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::core::content_type::ContentType::Text,
            self.message(),
        )
    }
}

/// 请求验证器。
pub struct Validator<'a> {
    input: HashMap<String, String>,
    files: HashMap<String, UploadedFile>,
    rules: Vec<(String, Vec<Rule>)>,
    customs: Vec<(String, CustomRule<'a>)>,
}

impl<'a> Validator<'a> {
    pub fn from_request(req: &Request) -> Self {
        Self {
            input: collect_input(req),
            files: req.files().clone(),
            rules: Vec::new(),
            customs: Vec::new(),
        }
    }

    pub fn rules<F: Field>(mut self, field: F, rules: &[Rule]) -> Self {
        self.rules.push((field.name().to_string(), rules.to_vec()));
        self
    }

    pub fn rule<F: Field>(self, field: F, rule: Rule) -> Self {
        self.rules(field, &[rule])
    }

    /// 自定义验证：返回稳定码，例如 `Err("title.spam".into())`。
    pub fn custom<F, C>(mut self, field: F, f: C) -> Self
    where
        F: Field,
        C: Fn(&str, &HashMap<String, String>) -> Result<(), String> + Send + Sync + 'a,
    {
        self.customs.push((field.name().to_string(), Box::new(f)));
        self
    }

    pub fn validate(self) -> Result<Validated, ValidationError> {
        let mut errors: HashMap<String, Vec<String>> = HashMap::new();

        if self.input.contains_key("__nx_error") {
            errors
                .entry("_".into())
                .or_default()
                .push("validation.payload".into());
            return Err(ValidationError { errors });
        }

        for (field, rules) in &self.rules {
            let value = self.input.get(field).map(String::as_str).unwrap_or("");
            for rule in rules {
                if let Err(msg) = rule.check(field, value, &self.input, &self.files) {
                    errors.entry(field.clone()).or_default().push(msg);
                    break; // 同字段遇错即停（Laravel 默认也可 stop on first）
                }
            }
        }

        for (field, custom) in &self.customs {
            let value = self.input.get(field).map(String::as_str).unwrap_or("");
            if let Err(msg) = custom(value, &self.input) {
                errors.entry(field.clone()).or_default().push(msg);
            }
        }

        if errors.is_empty() {
            Ok(Validated {
                values: self.input,
                files: self.files,
            })
        } else {
            Err(ValidationError { errors })
        }
    }
}

fn collect_input(req: &Request) -> HashMap<String, String> {
    // 含 `_nx` AES-GCM 密文时自动解密合并（见 server_fn::SEAL_FIELD）
    match crate::core::server_fn::expand_input_map(req) {
        Ok(map) => map,
        Err(msg) => {
            tracing::debug!(error = %msg, "request input expansion failed");
            let mut map = HashMap::new();
            map.insert("__nx_error".into(), String::new());
            map
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_error_uses_installed_hook() {
        install_error_translator(|code| {
            if code == "username.required" {
                "请填写用户名".into()
            } else {
                code.into()
            }
        });
        assert_eq!(translate_error("username.required"), "请填写用户名");
        clear_error_translator();
        assert_eq!(translate_error("username.required"), "username.required");
    }
}
