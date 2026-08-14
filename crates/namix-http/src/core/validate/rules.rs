use std::collections::HashMap;

use super::presence::presence_exists;
use crate::core::upload::UploadedFile;

/// 内置验证规则。字段用 enum（`FormField`）；更高定制用 `Validator::custom`。
#[derive(Clone, Debug)]
pub enum Rule {
    /// 非空
    Required,
    /// 邮箱
    Email,
    /// 最小长度
    Min(usize),
    /// 最大长度
    Max(usize),
    /// 长度区间（含端点）
    Between(usize, usize),
    /// 可解析为数字
    Numeric,
    /// 整数
    Integer,
    /// 纯数字位数（如验证码）
    Digits(usize),
    /// 字母数字
    AlphaNum,
    /// http(s) URL
    Url,
    /// 当前站点内的绝对路径（拒绝 `https://…`、`//host` 与反斜杠变体）
    LocalPath,
    /// 布尔：`1/0/true/false/yes/no/on/off`
    Boolean,
    /// 必须为接受值：`1/true/yes/on`
    Accepted,
    /// 必须为拒绝值：`0/false/no/off`
    Declined,
    /// 等于某字符串
    Eq(&'static str),
    /// 不等于
    NotEq(&'static str),
    /// 前缀
    StartsWith(&'static str),
    /// 后缀
    EndsWith(&'static str),
    /// 需存在 `{field}_confirmation` 且相等
    Confirmed,
    /// 与另一字段相等
    Same(&'static str),
    /// 白名单
    In(&'static [&'static str]),
    /// 黑名单
    NotIn(&'static [&'static str]),
    /// 极简正则（`^` `$` `\d+` `[a-zA-Z]+` 与字面量）
    Regex(&'static str),
    /// 必须上传文件
    File,
    /// 图片（按 MIME 或扩展名）
    Image,
    /// 允许的扩展名 / MIME（如 `png`、`jpg`）
    Mimes(&'static [&'static str]),
    /// 上传体积上限
    MaxBytes(usize),
    /// 表中该列不得已有此值（空值跳过）
    Unique {
        table: &'static str,
        column: &'static str,
    },
    /// unique，但忽略某一行（更新资料时排除自己）
    UniqueIgnore {
        table: &'static str,
        column: &'static str,
        except_column: &'static str,
        except_id: String,
    },
    /// 表中该列必须已有此值（空值跳过）
    Exists {
        table: &'static str,
        column: &'static str,
    },
}

impl Rule {
    pub fn unique(table: &'static str, column: &'static str) -> Self {
        Self::Unique { table, column }
    }

    pub fn unique_ignore(
        table: &'static str,
        column: &'static str,
        except_id: impl Into<String>,
    ) -> Self {
        Self::UniqueIgnore {
            table,
            column,
            except_column: "id",
            except_id: except_id.into(),
        }
    }

    pub fn unique_ignore_col(
        table: &'static str,
        column: &'static str,
        except_column: &'static str,
        except_id: impl Into<String>,
    ) -> Self {
        Self::UniqueIgnore {
            table,
            column,
            except_column,
            except_id: except_id.into(),
        }
    }

    pub fn exists(table: &'static str, column: &'static str) -> Self {
        Self::Exists { table, column }
    }

    pub fn check(
        &self,
        field: &str,
        value: &str,
        all: &HashMap<String, String>,
        files: &HashMap<String, UploadedFile>,
    ) -> Result<(), String> {
        match self {
            Rule::Required => {
                if value.trim().is_empty() && !files.contains_key(field) {
                    fail(field, "required")
                } else {
                    Ok(())
                }
            }
            Rule::Email => {
                if value.is_empty() {
                    return Ok(());
                }
                let ok = value.contains('@')
                    && value.split('@').count() == 2
                    && value.split_once('@').is_some_and(|(u, d)| {
                        !u.is_empty() && d.contains('.') && !d.starts_with('.') && !d.ends_with('.')
                    });
                if ok { Ok(()) } else { fail(field, "email") }
            }
            Rule::Min(n) => {
                if value.is_empty() || value.chars().count() >= *n {
                    Ok(())
                } else {
                    fail(field, "min")
                }
            }
            Rule::Max(n) => {
                if value.chars().count() <= *n {
                    Ok(())
                } else {
                    fail(field, "max")
                }
            }
            Rule::Between(min, max) => {
                let len = value.chars().count();
                if value.is_empty() || (len >= *min && len <= *max) {
                    Ok(())
                } else {
                    fail(field, "between")
                }
            }
            Rule::Numeric => {
                if value.is_empty() || value.parse::<f64>().is_ok() {
                    Ok(())
                } else {
                    fail(field, "numeric")
                }
            }
            Rule::Integer => {
                if value.is_empty() || value.parse::<i64>().is_ok() {
                    Ok(())
                } else {
                    fail(field, "integer")
                }
            }
            Rule::Digits(n) => {
                if value.is_empty()
                    || (value.len() == *n && value.chars().all(|c| c.is_ascii_digit()))
                {
                    Ok(())
                } else {
                    fail(field, "digits")
                }
            }
            Rule::AlphaNum => {
                if value.is_empty() || value.chars().all(|c| c.is_ascii_alphanumeric()) {
                    Ok(())
                } else {
                    fail(field, "alpha_num")
                }
            }
            Rule::Url => {
                if value.is_empty() || value.starts_with("http://") || value.starts_with("https://")
                {
                    Ok(())
                } else {
                    fail(field, "url")
                }
            }
            Rule::LocalPath => {
                if value.is_empty() || crate::core::request::is_local_path(value) {
                    Ok(())
                } else {
                    fail(field, "local_path")
                }
            }
            Rule::Boolean => {
                if value.is_empty() || is_boolish(value) {
                    Ok(())
                } else {
                    fail(field, "boolean")
                }
            }
            Rule::Accepted => {
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                ) {
                    Ok(())
                } else {
                    fail(field, "accepted")
                }
            }
            Rule::Declined => {
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                ) {
                    Ok(())
                } else {
                    fail(field, "declined")
                }
            }
            Rule::Eq(expected) => {
                if value.is_empty() || value == *expected {
                    Ok(())
                } else {
                    fail(field, "eq")
                }
            }
            Rule::NotEq(denied) => {
                if value.is_empty() || value != *denied {
                    Ok(())
                } else {
                    fail(field, "invalid")
                }
            }
            Rule::StartsWith(prefix) => {
                if value.is_empty() || value.starts_with(prefix) {
                    Ok(())
                } else {
                    fail(field, "starts_with")
                }
            }
            Rule::EndsWith(suffix) => {
                if value.is_empty() || value.ends_with(suffix) {
                    Ok(())
                } else {
                    fail(field, "ends_with")
                }
            }
            Rule::Confirmed => {
                let other = all
                    .get(&format!("{field}_confirmation"))
                    .map(String::as_str)
                    .unwrap_or("");
                if value == other {
                    Ok(())
                } else {
                    fail(field, "confirmed")
                }
            }
            Rule::Same(other_field) => {
                let other = all.get(*other_field).map(String::as_str).unwrap_or("");
                if value == other {
                    Ok(())
                } else {
                    fail(field, "same")
                }
            }
            Rule::In(allowed) => {
                if value.is_empty() || allowed.contains(&value) {
                    Ok(())
                } else {
                    fail(field, "invalid")
                }
            }
            Rule::NotIn(denied) => {
                if value.is_empty() || !denied.contains(&value) {
                    Ok(())
                } else {
                    fail(field, "invalid")
                }
            }
            Rule::Regex(pat) => {
                if value.is_empty() {
                    return Ok(());
                }
                match regex::Regex::new(pat) {
                    Ok(re) if re.is_match(value) => Ok(()),
                    Ok(_) => fail(field, "regex"),
                    Err(_) => fail(field, "regex"),
                }
            }
            Rule::File => {
                if files.get(field).is_some_and(|file| !file.is_empty()) {
                    Ok(())
                } else {
                    fail(field, "file")
                }
            }
            Rule::Image => match files.get(field) {
                None => Ok(()),
                Some(file) if file.is_empty() => Ok(()),
                Some(file) if file.is_image() => Ok(()),
                Some(_) => fail(field, "image"),
            },
            Rule::Mimes(allowed) => match files.get(field) {
                None => Ok(()),
                Some(file) if file.is_empty() => Ok(()),
                Some(file) if file.matches_mime(allowed) => Ok(()),
                Some(_) => fail(field, "mimes"),
            },
            Rule::MaxBytes(max) => match files.get(field) {
                None => Ok(()),
                Some(file) if file.size() <= *max => Ok(()),
                Some(_) => fail(field, "max_bytes"),
            },
            Rule::Unique { table, column } => check_unique(field, value, table, column, None),
            Rule::UniqueIgnore {
                table,
                column,
                except_column,
                except_id,
            } => check_unique(
                field,
                value,
                table,
                column,
                Some((*except_column, except_id.as_str())),
            ),
            Rule::Exists { table, column } => {
                if value.trim().is_empty() {
                    return Ok(());
                }
                match presence_exists(table, column, value, None) {
                    Ok(true) => Ok(()),
                    Ok(false) => fail(field, "exists"),
                    Err(_) => fail(field, "presence"),
                }
            }
        }
    }
}

fn fail(field: &str, rule: &str) -> Result<(), String> {
    Err(format!("{field}.{rule}"))
}

fn check_unique(
    field: &str,
    value: &str,
    table: &str,
    column: &str,
    except: Option<(&str, &str)>,
) -> Result<(), String> {
    if value.trim().is_empty() {
        return Ok(());
    }
    match presence_exists(table, column, value, except) {
        Ok(true) => fail(field, "taken"),
        Ok(false) => Ok(()),
        Err(_) => fail(field, "presence"),
    }
}

fn is_boolish(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "1" | "0" | "true" | "false" | "yes" | "no" | "on" | "off"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(rule: &Rule, field: &str, value: &str) -> Result<(), String> {
        rule.check(field, value, &HashMap::new(), &HashMap::new())
    }

    #[test]
    fn local_path_rule_allows_empty_or_internal_paths() {
        assert!(check(&Rule::LocalPath, "redirect", "").is_ok());
        assert!(check(&Rule::LocalPath, "redirect", "/account?tab=billing").is_ok());
        assert_eq!(
            check(&Rule::LocalPath, "redirect", "//example.test").unwrap_err(),
            "redirect.local_path"
        );
        assert_eq!(
            check(&Rule::Required, "username", "").unwrap_err(),
            "username.required"
        );
        assert_eq!(
            check(&Rule::Min(3), "username", "ab").unwrap_err(),
            "username.min"
        );
    }

    #[test]
    fn unique_skips_empty_and_rejects_taken() {
        use std::sync::Arc;

        use crate::core::validate::presence::{
            PRESENCE_TEST_LOCK, PresenceVerifier, clear_presence_verifier,
            install_presence_verifier,
        };

        struct Taken;
        impl PresenceVerifier for Taken {
            fn exists(
                &self,
                _table: &str,
                _column: &str,
                value: &str,
                _except: Option<(&str, &str)>,
            ) -> Result<bool, String> {
                Ok(value == "taken@namix.local")
            }
        }

        let _lock = PRESENCE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        install_presence_verifier(Arc::new(Taken));
        assert!(check(&Rule::unique("profiles", "email"), "email", "").is_ok());
        assert!(
            check(
                &Rule::unique("profiles", "email"),
                "email",
                "free@namix.local"
            )
            .is_ok()
        );
        assert_eq!(
            check(
                &Rule::unique("profiles", "email"),
                "email",
                "taken@namix.local"
            )
            .unwrap_err(),
            "email.taken"
        );
        assert!(
            check(
                &Rule::exists("users", "username"),
                "username",
                "taken@namix.local"
            )
            .is_ok()
        );
        clear_presence_verifier();
    }
}
