//! `nx make model|validator|controller` — 按单/多应用落到正确目录。
//!
//! 多应用约定：
//! ```text
//! common/     models · services · validators(共享) · seeders
//! www|user|admin/   controllers · routes · middleware · validators(端专属)
//! ```
//!
//! ```bash
//! nx make model Article                      # → common/models
//! nx make validator Login                    # → common/validators
//! nx make validator Checkout --app user      # → user/validators
//! nx make controller Home --app user         # → user/controllers
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use crate::project::{Project, normalize_type_name};
use crate::scope::{MakeKind, Scope, resolve_for_make};
use crate::template::Mode;

pub fn model(
    project: &Project,
    name: &str,
    with_migration: bool,
    app: Option<&str>,
) -> Result<(), String> {
    let _scope = resolve_for_make(project.mode, MakeKind::Model, app)?;
    let (pascal, snake) = normalize_type_name(name)?;
    let dir = project.models_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    ensure_named_feature_marker(&dir, "models")?;
    let path = dir.join(format!("{snake}.rs"));
    if path.exists() {
        return Err(format!("已存在: {}", path.display()));
    }

    let body = format!(
        r#"//! {pascal} → 表 `{table}`

#[derive(Clone, Debug, toasty::Model)]
pub struct {pascal} {{
    #[key]
    #[auto]
    pub id: u64,

    pub title: String,
}}
"#,
        table = pluralize(&snake),
    );
    write_new(&path, &body)?;
    register_model(&project.registry_rs(), &pascal, &snake)?;

    println!("✓ model     {}", path.display());
    println!("  scope     {}", scope_hint(project, &Scope::Common));
    println!("  registry  已注册 {pascal}");
    println!("  配置      namix.toml [features].models = true");
    println!("            [database] enabled = true + Cargo namix feature（sqlite/…）");

    if with_migration {
        println!();
        crate::migrate::generate(project)?;
    }
    Ok(())
}

pub fn validator(project: &Project, name: &str, app: Option<&str>) -> Result<(), String> {
    let scope = resolve_for_make(project.mode, MakeKind::Validator, app)?;
    let (mut pascal, mut snake) = normalize_type_name(name)?;
    if !snake.ends_with("_form") {
        snake = format!("{snake}_form");
        pascal = format!("{pascal}Form");
    } else if !pascal.ends_with("Form") {
        pascal = format!("{pascal}Form");
    }

    let dir = validators_dir(project, &scope)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    if matches!(scope, Scope::Common) {
        ensure_named_feature_marker(&dir, "validators")?;
    }

    let path = dir.join(format!("{snake}.rs"));
    if path.exists() {
        return Err(format!("已存在: {}", path.display()));
    }

    let field = snake
        .trim_end_matches("_form")
        .split('_')
        .next()
        .unwrap_or("name");
    let field_pascal = normalize_type_name(field)
        .map(|(p, _)| p)
        .unwrap_or_else(|_| "Name".into());
    let request_ty = format!(
        "{}Request",
        pascal.strip_suffix("Form").unwrap_or(pascal.as_str())
    );
    let route_hint = snake.trim_end_matches("_form");

    let body = validator_source(&pascal, &request_ty, &field_pascal, field, route_hint);
    write_new(&path, &body)?;
    register_src_module(&dir, &snake)?;
    println!("✓ validator {}", path.display());
    println!("  scope     {}", scope_hint(project, &scope));
    println!(
        "  用法      {path}::from_values(&req) 或提取器 `form: {request_ty}`",
        path = validator_mod_path(project, &scope, &snake),
    );
    println!("  配置      namix.toml [features].validators = true");
    if matches!(scope, Scope::App(_)) {
        println!("  提示      cargo check 后 namix-build 会挂上该端 validators 模块");
    }
    Ok(())
}

fn validator_source(
    form_enum: &str,
    request_ty: &str,
    field_pascal: &str,
    field: &str,
    route_hint: &str,
) -> String {
    format!(
        r#"//! {form_enum} — Form Request。

use namix::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum {form_enum} {{
    #[field = "{field}"]
    {field_pascal},
}}

#[derive(Clone, Debug)]
pub struct {request_ty} {{
    pub {field}: String,
}}

impl FormRequest for {request_ty} {{
    fn redirect_to() -> FormRedirect {{
        FormRedirect::Named("{route_hint}")
    }}

    fn from_values(req: &Request) -> Result<Self, ValidationError> {{
        let v = req
            .validator()
            .rules({form_enum}::{field_pascal}, &[Rule::Required, Rule::Between(1, 64)])
            .validate()?;

        Ok(Self {{
            {field}: v.get({form_enum}::{field_pascal}).to_string(),
        }})
    }}
}}
"#
    )
}

pub fn controller(project: &Project, name: &str, app: Option<&str>) -> Result<(), String> {
    let scope = resolve_for_make(project.mode, MakeKind::Controller, app)?;
    let (pascal, snake) = normalize_type_name(name)?;
    let dir = controllers_dir(project, &scope)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{snake}.rs"));
    if path.exists() {
        return Err(format!("已存在: {}", path.display()));
    }

    let body = format!(
        r#"use namix::prelude::*;

pub async fn index(_req: Request) -> Response {{
    text("{pascal}")
}}
"#
    );
    write_new(&path, &body)?;
    register_src_module(&dir, &snake)?;
    println!("✓ controller {}", path.display());
    println!("  scope      {}", scope_hint(project, &scope));
    println!(
        "  下一步     在 routes 里 `use {}::{{index}}`，并加入：",
        controller_mod_path(project, &scope, &snake)
    );
    println!("             GET \"/{snake}\" => {snake}::index, name: \"{snake}\",");
    if project.feature_enabled("pages") {
        println!("  页面       带 ViewData + TSX 请用 `nx make page {pascal}`");
    }
    Ok(())
}

pub fn page(project: &Project, name: &str, app: Option<&str>) -> Result<(), String> {
    if !project.feature_enabled("pages") {
        return Err(
            "需要 [features].pages = true。打开后用 `nx make page Notes` 一次生成控制器 + 页面。"
                .into(),
        );
    }
    let scope = resolve_for_make(project.mode, MakeKind::Page, app)?;
    let (pascal, snake) = normalize_type_name(name)?;
    let dir = controllers_dir(project, &scope)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let controller_path = dir.join(format!("{snake}.rs"));
    let ext = page_ext(project);
    let tsx_path = project
        .src_dir()
        .join("views/pages")
        .join(format!("{snake}.{ext}"));
    if controller_path.exists() {
        return Err(format!("已存在: {}", controller_path.display()));
    }
    if tsx_path.exists() {
        return Err(format!("已存在: {}", tsx_path.display()));
    }

    let page_ty = format!("{pascal}Page");
    write_new(
        &controller_path,
        &page_controller_source(&pascal, &snake, &page_ty),
    )?;
    register_src_module(&dir, &snake)?;
    write_new(&tsx_path, &page_tsx_source(&page_ty, &pascal))?;
    write_view_data_stub(project, &page_ty)?;
    ensure_view_const(&project.src_dir(), &snake)?;
    ensure_lib_view_mod(&project.src_dir())?;
    ensure_app_prelude(project)?;

    println!("✓ page       {}", controller_path.display());
    println!("             {}", tsx_path.display());
    println!("  scope      {}", scope_hint(project, &scope));
    println!(
        "  下一步     在 routes 里 `use {}::{{index}}`，并加入：",
        controller_mod_path(project, &scope, &snake)
    );
    println!("             GET \"/{snake}\" => {snake}::index, name: \"{snake}\",");
    println!("  然后       cargo check -p app   # ViewData 覆盖 generated/{page_ty}.ts");
    Ok(())
}

/// 可选 HTML 错误页：`controllers/errors.rs` + `views/pages/errors.tsx`。
/// 不往 `web.rs` 自动挂钩；不注册则框架保持默认。
pub fn error(project: &Project, status: Option<&str>, app: Option<&str>) -> Result<(), String> {
    if !project.feature_enabled("pages") {
        return Err(
            "需要 [features].pages = true。打开后用 `nx make error` 生成 HTML 错误页。".into(),
        );
    }
    let status_hint = match status {
        Some(raw) => Some(parse_error_status(raw)?),
        None => None,
    };
    let scope = resolve_for_make(project.mode, MakeKind::Page, app)?;
    let dir = controllers_dir(project, &scope)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let controller_path = dir.join("errors.rs");
    let ext = page_ext(project);
    let tsx_path = project
        .src_dir()
        .join("views/pages")
        .join(format!("errors.{ext}"));
    if controller_path.exists() {
        return Err(format!("已存在: {}", controller_path.display()));
    }
    if tsx_path.exists() {
        return Err(format!("已存在: {}", tsx_path.display()));
    }

    write_new(&controller_path, &error_controller_source())?;
    register_src_module(&dir, "errors")?;
    write_new(&tsx_path, &error_tsx_source())?;
    write_view_data_stub_fields(
        project,
        "ErrorsPage",
        "  status: number\n  title: string\n  message: string\n",
    )?;
    ensure_view_const(&project.src_dir(), "errors")?;
    ensure_lib_view_mod(&project.src_dir())?;
    ensure_app_prelude(project)?;

    println!("✓ error      {}", controller_path.display());
    println!("             {}", tsx_path.display());
    println!("  scope      {}", scope_hint(project, &scope));
    if let Some(code) = status_hint {
        println!("  状态       骨架覆盖所有 HTML 错误；若只想要 {code}，在 routes 里写：");
        println!("             .error_page({code}, errors::page)");
    } else {
        println!("  下一步     在 routes() 末尾链式挂上（可选，不挂则保持框架默认）：");
        println!("             .error_page(404, errors::page)");
        println!("             .error_pages(errors::page)   // 403/500/429… 共用");
    }
    println!("  控制器     未匹配资源用 `return req.not_found();`，不要用自由函数 not_found()");
    Ok(())
}

fn parse_error_status(raw: &str) -> Result<u16, String> {
    let status: u16 = raw
        .parse()
        .map_err(|_| format!("状态码必须是数字，例如 404，收到 `{raw}`"))?;
    if !(400..600).contains(&status) {
        return Err(format!("状态码应在 400–599，收到 {status}"));
    }
    Ok(status)
}

fn error_controller_source() -> String {
    r#"//! 可选 HTML 错误页。不在 routes 上 `.error_pages(errors::page)` 则不会生效。

use crate::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct ErrorsPage {
    pub status: u16,
    pub title: String,
    pub message: String,
}

pub fn page(req: &Request, error: ErrorPage) -> Response {
    req.view(Page::Errors)
        .ssr()
        .title(error.reason())
        .data(ErrorsPage {
            status: error.status,
            title: error.reason().to_string(),
            message: error.message,
        })
        .render()
}
"#
    .into()
}

fn error_tsx_source() -> String {
    r#"import type { ErrorsPage } from '../generated/ErrorsPage'
import { Head, Link, route } from '../namix'
import type { PageProps } from '../types'

type Props = PageProps<ErrorsPage>

export default function Errors({ status, title, message }: Props) {
  return (
    <main className="min-h-screen px-6 py-14">
      <Head title={title} />
      <p className="text-sm text-zinc-500">{status}</p>
      <h1 className="mt-2 text-3xl font-semibold tracking-tight">{title}</h1>
      <p className="mt-4 text-zinc-600">{message}</p>
      <p className="mt-8">
        <Link href={route.home()} className="text-sm text-teal-800 hover:text-teal-950">
          回到首页
        </Link>
      </p>
    </main>
  )
}
"#
    .into()
}

fn write_view_data_stub_fields(
    project: &Project,
    page_ty: &str,
    fields: &str,
) -> Result<(), String> {
    let dir = project.src_dir().join("views/generated");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{page_ty}.ts"));
    if path.exists() {
        return Ok(());
    }
    write_new(
        &path,
        &format!(
            "/* @generated stub — cargo check 后由 #[derive(ViewData)] 覆盖 */\n\
             export type {page_ty} = {{\n{fields}}}\n"
        ),
    )
}

fn page_ext(project: &Project) -> &'static str {
    let pages = project.src_dir().join("views/pages");
    if pages.join("home.jsx").exists() && !pages.join("home.tsx").exists() {
        "jsx"
    } else {
        "tsx"
    }
}

fn page_controller_source(pascal: &str, snake: &str, page_ty: &str) -> String {
    format!(
        r#"use crate::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, ViewData)]
#[serde(rename_all = "camelCase")]
pub struct {page_ty} {{
    pub title: String,
}}

pub async fn index(req: Request) -> Response {{
    req.view(Page::{pascal}) // 或 view::{snake}
        .island()
        .title("{pascal}")
        .data({page_ty} {{
            title: "{pascal}".into(),
        }})
        .render()
}}
"#
    )
}

fn page_tsx_source(page_ty: &str, pascal: &str) -> String {
    format!(
        r#"import type {{ {page_ty} }} from '../generated/{page_ty}'
import {{ Head, Link, route }} from '../namix'
import type {{ PageProps }} from '../types'

type Props = PageProps<{page_ty}>

export default function {pascal}({{ title }}: Props) {{
  return (
    <main className="min-h-screen px-6 py-14">
      <Head title={{title}} />
      <h1 className="text-3xl font-semibold tracking-tight">{{title}}</h1>
      <p className="mt-6">
        <Link href={{route.home()}} className="text-sm text-teal-800 hover:text-teal-950">
          回首页
        </Link>
      </p>
    </main>
  )
}}
"#
    )
}

fn write_view_data_stub(project: &Project, page_ty: &str) -> Result<(), String> {
    let dir = project.src_dir().join("views/generated");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{page_ty}.ts"));
    if path.exists() {
        return Ok(());
    }
    write_new(
        &path,
        &format!(
            "/* @generated stub — cargo check 后由 #[derive(ViewData)] 覆盖 */\n\
             export type {page_ty} = {{\n  title: string\n}}\n"
        ),
    )
}

fn snake_to_pascal(snake: &str) -> String {
    snake
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

fn ensure_view_const(src_dir: &Path, snake: &str) -> Result<(), String> {
    let path = src_dir.join("view.rs");
    let pascal = snake_to_pascal(snake);
    let line = format!("pub const {snake}: &str = \"{snake}\";");
    if path.exists() {
        let src = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if src.contains(&format!("pub const {snake}:")) {
            return Ok(());
        }
        let mut body = src;
        if !body.ends_with('\n') {
            body.push('\n');
        }
        body.push_str(&line);
        body.push('\n');
        return fs::write(&path, body).map_err(|e| e.to_string());
    }
    write_new(
        &path,
        &format!(
            "// @generated by namix-build — DO NOT EDIT\n\
             //! 页面名：`req.view(Page::{pascal})` / `req.view(view::{snake})` 与 `views/pages/{snake}.tsx` 对齐。\n\n\
             #![allow(non_upper_case_globals)]\n\n\
             #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\n\
             pub enum Page {{\n    {pascal},\n}}\n\n\
             impl Page {{\n    pub fn as_str(self) -> &'static str {{\n        match self {{\n            Self::{pascal} => \"{snake}\",\n        }}\n    }}\n}}\n\n\
             impl From<Page> for String {{\n    fn from(page: Page) -> Self {{\n        page.as_str().into()\n    }}\n}}\n\n\
             impl AsRef<str> for Page {{\n    fn as_ref(&self) -> &str {{\n        self.as_str()\n    }}\n}}\n\n\
             {line}\n"
        ),
    )
}

fn ensure_app_prelude(project: &Project) -> Result<(), String> {
    let src_dir = project.src_dir();
    let path = src_dir.join("prelude.rs");
    if !path.exists() {
        let body = match project.mode {
            Mode::Single => {
                "//! 业务侧一键导入：框架 prelude + 本应用的 `AppRoute` / `Page`。\n\
                 //!\n\
                 //! `Route` 仍是注册路由用的 `Route::get`；命名路由枚举叫 [`AppRoute`]，避免撞名。\n\n\
                 pub use namix::prelude::*;\n\
                 pub use crate::route::{self, AppRoute};\n\
                 pub use crate::view::{self, Page};\n"
            }
            Mode::Multi => {
                "//! 业务侧一键导入。多应用下命名路由在 `route::user::login` / `route::user::AppRoute`。\n\
                 //!\n\
                 //! `Route` 仍是注册路由用的 `Route::get`。\n\n\
                 pub use namix::prelude::*;\n\
                 pub use crate::route;\n\
                 pub use crate::view::{self, Page};\n"
            }
        };
        write_new(&path, body)?;
    }
    ensure_lib_prelude_mod(&src_dir)
}

fn ensure_lib_prelude_mod(src_dir: &Path) -> Result<(), String> {
    let path = src_dir.join("lib.rs");
    if !path.exists() {
        return Ok(());
    }
    let src = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    if src.contains("pub mod prelude;") {
        return Ok(());
    }
    let mut body = src;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str("pub mod prelude;\n");
    fs::write(path, body).map_err(|e| e.to_string())
}

fn ensure_lib_view_mod(src_dir: &Path) -> Result<(), String> {
    let path = src_dir.join("lib.rs");
    if !path.exists() {
        return Ok(());
    }
    let src = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    if src.contains("pub mod view;") {
        return Ok(());
    }
    let mut body = src;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str("pub mod view;\n");
    fs::write(path, body).map_err(|e| e.to_string())
}

fn ensure_package_module(src_dir: &Path, dir_name: &str) -> Result<(), String> {
    let path = src_dir.join("namix_modules.rs");
    if !path.exists() {
        return Ok(());
    }
    let src = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let line = format!("pub mod {dir_name};");
    if src.contains(&line) {
        return Ok(());
    }
    let mut body = src;
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&line);
    body.push('\n');
    fs::write(path, body).map_err(|e| e.to_string())
}

fn register_src_module(dir: &Path, snake: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mod_rs = dir.join("mod.rs");
    let mut modules = Vec::new();
    if mod_rs.exists() {
        let src = fs::read_to_string(&mod_rs).map_err(|e| e.to_string())?;
        for line in src.lines() {
            let line = line.trim();
            if let Some(name) = line
                .strip_prefix("pub mod ")
                .and_then(|rest| rest.strip_suffix(';'))
            {
                let name = name.trim();
                if !name.is_empty() {
                    modules.push(name.to_string());
                }
            }
        }
    }
    if !modules.iter().any(|name| name == snake) {
        modules.push(snake.to_string());
    }
    modules.sort();
    modules.dedup();
    let mut body = String::from(
        "// @generated by namix-build — DO NOT EDIT\n\
         // 新增 .rs 后 cargo check 自动注册。\n\n",
    );
    for name in &modules {
        body.push_str("pub mod ");
        body.push_str(name);
        body.push_str(";\n");
    }
    fs::write(mod_rs, body).map_err(|e| e.to_string())
}

fn validators_dir(project: &Project, scope: &Scope) -> Result<PathBuf, String> {
    Ok(match (project.mode, scope) {
        (Mode::Single, _) => project.src_dir().join("validators"),
        (Mode::Multi, Scope::Common) => project.src_dir().join("common/validators"),
        (Mode::Multi, Scope::App(app)) => project.src_dir().join(app).join("validators"),
    })
}

fn controllers_dir(project: &Project, scope: &Scope) -> Result<PathBuf, String> {
    match (project.mode, scope) {
        (Mode::Single, _) => Ok(project.src_dir().join("controllers")),
        (Mode::Multi, Scope::App(app)) => Ok(project.src_dir().join(app).join("controllers")),
        (Mode::Multi, Scope::Common) => Err("controller 不能写在 common".into()),
    }
}

fn scope_hint(project: &Project, scope: &Scope) -> String {
    match project.mode {
        Mode::Single => "single".into(),
        Mode::Multi => format!("multi / {}", scope.label()),
    }
}

fn validator_mod_path(project: &Project, scope: &Scope, snake: &str) -> String {
    match (project.mode, scope) {
        (Mode::Single, _) => format!("crate::validators::{snake}"),
        (Mode::Multi, Scope::Common) => format!("crate::common::validators::{snake}"),
        (Mode::Multi, Scope::App(app)) => format!("crate::{app}::validators::{snake}"),
    }
}

fn controller_mod_path(project: &Project, scope: &Scope, snake: &str) -> String {
    match (project.mode, scope) {
        (Mode::Single, _) => format!("crate::controllers::{snake}"),
        (Mode::Multi, Scope::App(app)) => format!("crate::{app}::controllers::{snake}"),
        (Mode::Multi, Scope::Common) => format!("crate::controllers::{snake}"),
    }
}

fn ensure_named_feature_marker(dir: &Path, feature: &str) -> Result<(), String> {
    let marker = dir.join(".namix-feature");
    if !marker.exists() {
        fs::write(
            &marker,
            format!("feature = \"{feature}\"\n# managed by namix-build — do not remove\n"),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn pluralize(snake: &str) -> String {
    if snake.ends_with('s') {
        snake.to_string()
    } else if snake.ends_with('y') && snake.len() > 1 {
        format!("{}ies", &snake[..snake.len() - 1])
    } else {
        format!("{snake}s")
    }
}

fn write_new(path: &Path, body: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, body).map_err(|e| e.to_string())
}

fn register_model(registry: &Path, pascal: &str, snake: &str) -> Result<(), String> {
    if !registry.exists() {
        let body = format!(
            r#"//! 全部 Toasty 模型注册表。

use super::{snake}::{pascal};

pub fn model_set() -> toasty::ModelSet {{
    toasty::models!({pascal})
}}
"#
        );
        return write_new(registry, &body);
    }

    let mut src = fs::read_to_string(registry).map_err(|e| e.to_string())?;
    if src.contains(&format!("use super::{snake}::{pascal}")) {
        return Ok(());
    }

    let use_line = format!("use super::{snake}::{pascal};\n");
    if let Some(idx) = src.find("pub fn model_set") {
        src.insert_str(idx, &use_line);
    } else {
        src.push_str(&use_line);
    }

    if let Some(start) = src.find("toasty::models!(") {
        let rest = &src[start + "toasty::models!(".len()..];
        if let Some(end_rel) = rest.find(')') {
            let inner = rest[..end_rel].trim();
            if inner.split(',').any(|p| p.trim() == pascal) {
                fs::write(registry, &src).map_err(|e| e.to_string())?;
                return Ok(());
            }
            let new_inner = if inner.is_empty() {
                pascal.to_string()
            } else {
                format!("{inner}, {pascal}")
            };
            let before = &src[..start + "toasty::models!(".len()];
            let after = &src[start + "toasty::models!(".len() + end_rel..];
            src = format!("{before}{new_inner}{after}");
        }
    } else {
        return Err("registry.rs 中找不到 toasty::models!(...)".into());
    }

    fs::write(registry, src).map_err(|e| e.to_string())
}

pub fn resource(project: &Project, name: &str, app: Option<&str>) -> Result<(), String> {
    let scope = resolve_for_make(project.mode, MakeKind::Resource, app)?;
    let (pascal, snake) = normalize_type_name(name)?;
    let dir = controllers_dir(project, &scope)?;
    let path = dir.join(format!("{}_controller.rs", snake));
    let body = resource_source(&pascal, &snake);
    write_generated(&path, &body, "resource")?;
    register_src_module(&dir, &format!("{snake}_controller"))?;
    println!("  下一步     router.merge(resource(\"{snake}\", {pascal}Controller))");
    println!("             update 同时绑 PATCH 与 PUT");
    Ok(())
}

fn resource_source(pascal: &str, snake: &str) -> String {
    let body = format!(
        r#"use namix::prelude::*;

#[derive(Clone)]
pub struct {pascal}Controller;

impl ResourceController for {pascal}Controller {{
    fn index(&self, _req: Request) -> ResourceFuture<'_> {{
        Box::pin(async move {{ Ok(text("{snake}.index")) }})
    }}

    fn show(&self, req: Request) -> ResourceFuture<'_> {{
        Box::pin(async move {{
            Ok(text(format!("{snake}.show:{{}}", req.param_or("id", ""))))
        }})
    }}

    // store / update / destroy default to 405 until implemented.
}}

// Register: router.merge(resource("{snake}", {pascal}Controller))
// update is bound to both PATCH and PUT.
"#
    );
    body
}

pub fn policy(project: &Project, name: &str) -> Result<(), String> {
    let (pascal, snake) = normalize_type_name(name)?;
    let path = project
        .src_dir()
        .join("policies")
        .join(format!("{}_policy.rs", snake));
    let body = policy_source(&pascal, &snake);
    write_generated(&path, &body, "policy")?;
    let dir = project.src_dir().join("policies");
    register_src_module(&dir, &format!("{snake}_policy"))?;
    ensure_package_module(&project.src_dir(), "policies")?;
    println!("  用法       authorize(&user, &{pascal}Policy, Ability::Update, Some(&row))?;");
    Ok(())
}

fn policy_source(pascal: &str, snake: &str) -> String {
    format!(
        r#"use namix::prelude::*;

use crate::models::{snake}::{pascal};
use crate::services::session::LoginUser;

pub struct {pascal}Policy;

impl Policy<LoginUser, {pascal}> for {pascal}Policy {{
    fn allows(&self, actor: &LoginUser, ability: Ability, resource: Option<&{pascal}>) -> bool {{
        match ability {{
            Ability::Create | Ability::ViewAny => true,
            Ability::View | Ability::Update | Ability::Delete => {{
                // Compare the session actor to the **database** record.
                // Never trust a form/JSON `user_id`.
                resource.is_some_and(|item| item.user_id == actor.id)
            }}
        }}
    }}
}}
"#
    )
}

pub fn job(project: &Project, name: &str) -> Result<(), String> {
    let (pascal, snake) = normalize_type_name(name)?;
    let path = project.src_dir().join("jobs").join(format!("{snake}.rs"));
    let body = format!(
        r#"use namix::prelude::*;
use serde::{{Deserialize, Serialize}};

#[derive(Serialize, Deserialize)]
pub struct {pascal} {{
    pub user_id: u64,
}}

impl QueuedJob for {pascal} {{
    const NAME: &'static str = "{snake}";
    fn handle(self) -> JobFuture {{
        Box::pin(async move {{
            namix::log::info!("{snake} user_id={{}}", self.user_id);
            Ok(())
        }})
    }}
}}
"#
    );
    write_generated(&path, &body, "job")?;
    register_src_module(&project.src_dir().join("jobs"), &snake)?;
    ensure_package_module(&project.src_dir(), "jobs")?;
    println!("  用法       register_job::<{pascal}>(); dispatch_job({pascal} {{ user_id: 1 }})?;");
    println!("  worker     nx work");
    Ok(())
}

pub fn mail(project: &Project, name: &str) -> Result<(), String> {
    let (pascal, snake) = normalize_type_name(name)?;
    let path = project.src_dir().join("mails").join(format!("{snake}.rs"));
    let body = format!(
        r#"use namix::prelude::*;

pub fn {snake}(to: &str) -> MailMessage {{ MailMessage::new(to, "{pascal}").text("{pascal} body") }}
"#
    );
    write_generated(&path, &body, "mail")?;
    register_src_module(&project.src_dir().join("mails"), &snake)?;
    ensure_package_module(&project.src_dir(), "mails")?;
    println!("  用法       Mail::send({snake}(\"user@example.test\"))?;");
    Ok(())
}

pub fn notification(project: &Project, name: &str) -> Result<(), String> {
    let (pascal, snake) = normalize_type_name(name)?;
    let path = project
        .src_dir()
        .join("notifications")
        .join(format!("{snake}.rs"));
    let body = format!(
        r#"use namix::prelude::*;

pub fn {snake}(recipient: &str) -> Notification {{
    Notification::new(NotificationChannel::Database, recipient, "{pascal}", "{pascal} body")
}}
"#
    );
    write_generated(&path, &body, "notification")?;
    register_src_module(&project.src_dir().join("notifications"), &snake)?;
    ensure_package_module(&project.src_dir(), "notifications")?;
    Ok(())
}

pub fn test(project: &Project, name: &str) -> Result<(), String> {
    let (pascal, snake) = normalize_type_name(name)?;
    let path = project
        .app_dir
        .join("tests")
        .join(format!("{snake}_test.rs"));
    let body = format!(
        r#"use namix::prelude::*;

#[tokio::test]
async fn {snake}_page_renders() {{
    let mut client = TestClient::new(app::routes::web::routes())
        .with_same_origin("http://127.0.0.1:3000")
        .expect("origin");
    let res = client.get("/").await;
    assert!(res.is_success(), "{pascal}: GET / should succeed");
}}
"#
    );
    write_generated(&path, &body, "test")?;
    println!("  运行       cargo test -p app --test {snake}_test");
    Ok(())
}

fn write_generated(path: &Path, body: &str, kind: &str) -> Result<(), String> {
    if path.exists() {
        return Err(format!("已存在: {}", path.display()));
    }
    write_new(path, body)?;
    println!("✓ {kind:<9} {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_template_uses_fallible_borrowing_futures() {
        let source = resource_source("Post", "post");

        assert!(source.contains("-> ResourceFuture<'_>"));
        assert!(source.contains("Box::pin(async move { Ok(text(\"post.index\")) })"));
        assert!(source.contains("Ok(text(format!(\"post.show:{}\""));
        assert!(source.contains("router.merge(resource(\"post\", PostController))"));
        assert!(!source.contains("-> ResourceFuture {"));
    }

    #[test]
    fn policy_template_binds_concrete_types_and_ownership() {
        let source = policy_source("Post", "post");
        assert!(source.contains("impl Policy<LoginUser, Post>"));
        assert!(source.contains("item.user_id == actor.id"));
        assert!(source.contains("use crate::models::post::Post;"));
        assert!(!source.contains("impl<Actor, Resource> Policy<Actor, Resource>"));
    }

    #[test]
    fn validator_template_implements_form_request() {
        let source = validator_source("LoginForm", "LoginRequest", "Username", "username", "login");
        assert!(source.contains("impl FormRequest for LoginRequest"));
        assert!(source.contains("FormRedirect::Named(\"login\")"));
        assert!(source.contains("from_values(req: &Request)"));
        assert!(!source.contains("pub fn validate(req: &Request)"));
    }

    #[test]
    fn page_templates_use_view_constants_and_generated_props() {
        let rust = page_controller_source("Notes", "notes", "NotesPage");
        assert!(rust.contains("req.view(Page::Notes)"));
        assert!(rust.contains("use crate::prelude::*"));
        assert!(rust.contains("#[derive(Debug, Clone, Serialize, ViewData)]"));
        assert!(rust.contains("pub struct NotesPage"));

        let tsx = page_tsx_source("NotesPage", "Notes");
        assert!(tsx.contains("import type { NotesPage } from '../generated/NotesPage'"));
        assert!(tsx.contains("import { Head, Link, route } from '../namix'"));
        assert!(tsx.contains("export default function Notes"));
        assert!(tsx.contains("PageProps<NotesPage>"));
        assert!(tsx.contains("route.home()"));
    }

    #[test]
    fn error_templates_register_optional_html_pages() {
        let rust = error_controller_source();
        assert!(rust.contains("req.view(Page::Errors)"));
        assert!(rust.contains("use crate::prelude::*"));
        assert!(rust.contains("pub fn page(req: &Request, error: ErrorPage)"));
        assert!(rust.contains(".ssr()"));
        assert!(!rust.contains("GET \"/errors\""));

        let tsx = error_tsx_source();
        assert!(tsx.contains("import type { ErrorsPage } from '../generated/ErrorsPage'"));
        assert!(tsx.contains("export default function Errors"));
        assert!(tsx.contains("route.home()"));
    }

    #[test]
    fn view_const_patch_is_idempotent() {
        let dir = std::env::temp_dir().join(format!(
            "namix-make-view-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("temp");
        ensure_view_const(&dir, "notes").expect("write");
        ensure_view_const(&dir, "notes").expect("again");
        let src = fs::read_to_string(dir.join("view.rs")).expect("read");
        assert_eq!(src.matches("pub const notes: &str = \"notes\";").count(), 1);
        assert!(src.contains("pub enum Page"));
        assert!(src.contains("Self::Notes => \"notes\""));
        let _ = fs::remove_dir_all(dir);
    }
}
