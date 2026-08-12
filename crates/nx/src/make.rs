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

    let body = format!(
        r#"//! {pascal} — 基础表单验证器。

use namix::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum {pascal} {{
    #[field = "{field}"]
    {field_pascal},
}}

pub fn validate(req: &Request) -> Result<Validated, ValidationError> {{
    req.validator()
        .rules(
            {pascal}::{field_pascal},
            &[Rule::Required, Rule::Between(1, 64)],
        )
        .validate()
}}
"#
    );
    write_new(&path, &body)?;
    println!("✓ validator {}", path.display());
    println!("  scope     {}", scope_hint(project, &scope));
    println!(
        "  用法      {}::validate(&req)",
        validator_mod_path(project, &scope, &snake)
    );
    println!("  配置      namix.toml [features].validators = true");
    if matches!(scope, Scope::App(_)) {
        println!("  提示      cargo check 后 namix-build 会挂上该端 validators 模块");
    }
    Ok(())
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
    println!("✓ controller {}", path.display());
    println!("  scope      {}", scope_hint(project, &scope));
    println!(
        "  用法       在 routes 里 `use {}::{{...}}`",
        controller_mod_path(project, &scope, &snake)
    );
    Ok(())
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
    write_generated(&path, &body, "resource")
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
}}

// routes! 之外可直接：router.merge(resource("{snake}", {pascal}Controller))
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
    let body = format!(
        r#"use namix::prelude::*;

pub struct {pascal}Policy;

impl<Actor, Resource> Policy<Actor, Resource> for {pascal}Policy {{
    fn allows(&self, _actor: &Actor, _ability: Ability, _resource: Option<&Resource>) -> bool {{ false }}
}}
"#
    );
    write_generated(&path, &body, "policy")
}

pub fn job(project: &Project, name: &str) -> Result<(), String> {
    let (pascal, snake) = normalize_type_name(name)?;
    let path = project.src_dir().join("jobs").join(format!("{snake}.rs"));
    let body = format!(
        r#"use namix::prelude::*;

pub struct {pascal};
impl Job for {pascal} {{
    fn name(&self) -> &'static str {{ "{snake}" }}
    fn handle(self: Box<Self>) -> JobFuture {{ Box::pin(async move {{ Ok(()) }}) }}
}}
"#
    );
    write_generated(&path, &body, "job")
}

pub fn mail(project: &Project, name: &str) -> Result<(), String> {
    let (pascal, snake) = normalize_type_name(name)?;
    let path = project.src_dir().join("mails").join(format!("{snake}.rs"));
    let body = format!(
        r#"use namix::prelude::*;

pub fn {snake}(to: &str) -> MailMessage {{ MailMessage::new(to, "{pascal}").text("{pascal} body") }}
"#
    );
    write_generated(&path, &body, "mail")
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
    write_generated(&path, &body, "notification")
}

pub fn test(project: &Project, name: &str) -> Result<(), String> {
    let (pascal, snake) = normalize_type_name(name)?;
    let path = project
        .app_dir
        .join("tests")
        .join(format!("{snake}_test.rs"));
    let body = format!(
        r#"// {pascal} integration test
// use namix::TestClient;

#[test]
fn {snake}_placeholder() {{ assert!(true); }}
"#
    );
    write_generated(&path, &body, "test")
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
        assert!(!source.contains("-> ResourceFuture {"));
    }
}
