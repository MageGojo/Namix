//! 多应用生成目标：`common` 或具体端 `www|user|admin`。

use crate::template::Mode;

/// 代码落点。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// 单应用扁平目录 / 多应用共享层
    Common,
    /// 多应用某一端
    App(String),
}

impl Scope {
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(Scope::Common);
        };
        let lower = raw.to_ascii_lowercase();
        match lower.as_str() {
            "common" | "shared" => Ok(Scope::Common),
            "www" | "user" | "admin" => Ok(Scope::App(lower)),
            other => Err(format!(
                "未知 scope/app `{other}`，可选: common, www, user, admin"
            )),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Scope::Common => "common".into(),
            Scope::App(a) => a.clone(),
        }
    }
}

/// 按产物类型解析默认 scope，并校验是否合法。
pub fn resolve_for_make(
    mode: Mode,
    kind: MakeKind,
    app_flag: Option<&str>,
) -> Result<Scope, String> {
    let scope = Scope::parse(app_flag)?;
    match mode {
        Mode::Single => {
            if matches!(scope, Scope::App(_)) {
                return Err("单应用没有 --app；验证器/实体都写在 src/ 扁平目录下".into());
            }
            Ok(Scope::Common)
        }
        Mode::Multi => match kind {
            MakeKind::Model => {
                if matches!(scope, Scope::App(_)) {
                    return Err(
                        "model 属于共享层，请用 `nx make model …`（写入 common/），不要加 --app"
                            .into(),
                    );
                }
                Ok(Scope::Common)
            }
            MakeKind::Validator => Ok(scope), // common 或某端
            MakeKind::Resource => match scope {
                Scope::App(_) => Ok(scope),
                Scope::Common => Err("多应用下 `resource` 必须指定端：`--app user`".into()),
            },
            MakeKind::Controller => match scope {
                Scope::App(_) => Ok(scope),
                Scope::Common => {
                    Err("多应用下 `controller` 必须指定端：`--app user`（或 www / admin）".into())
                }
            },
        },
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MakeKind {
    Model,
    Validator,
    Controller,
    Resource,
}

#[allow(dead_code)]
impl MakeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MakeKind::Model => "model",
            MakeKind::Validator => "validator",
            MakeKind::Controller => "controller",
            MakeKind::Resource => "resource",
        }
    }
}
