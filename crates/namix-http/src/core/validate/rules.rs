use std::collections::HashMap;

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
}

impl Rule {
    pub fn check(
        &self,
        field: &str,
        value: &str,
        all: &HashMap<String, String>,
    ) -> Result<(), String> {
        match self {
            Rule::Required => {
                if value.trim().is_empty() {
                    Err(format!("{field} is required"))
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
                if ok {
                    Ok(())
                } else {
                    Err(format!("{field} must be a valid email"))
                }
            }
            Rule::Min(n) => {
                if value.is_empty() || value.chars().count() >= *n {
                    Ok(())
                } else {
                    Err(format!("{field} must be at least {n} characters"))
                }
            }
            Rule::Max(n) => {
                if value.chars().count() <= *n {
                    Ok(())
                } else {
                    Err(format!("{field} must be at most {n} characters"))
                }
            }
            Rule::Between(min, max) => {
                let len = value.chars().count();
                if value.is_empty() || (len >= *min && len <= *max) {
                    Ok(())
                } else {
                    Err(format!(
                        "{field} must be between {min} and {max} characters"
                    ))
                }
            }
            Rule::Numeric => {
                if value.is_empty() || value.parse::<f64>().is_ok() {
                    Ok(())
                } else {
                    Err(format!("{field} must be numeric"))
                }
            }
            Rule::Integer => {
                if value.is_empty() || value.parse::<i64>().is_ok() {
                    Ok(())
                } else {
                    Err(format!("{field} must be an integer"))
                }
            }
            Rule::Digits(n) => {
                if value.is_empty()
                    || (value.len() == *n && value.chars().all(|c| c.is_ascii_digit()))
                {
                    Ok(())
                } else {
                    Err(format!("{field} must be {n} digits"))
                }
            }
            Rule::AlphaNum => {
                if value.is_empty() || value.chars().all(|c| c.is_ascii_alphanumeric()) {
                    Ok(())
                } else {
                    Err(format!("{field} must be alphanumeric"))
                }
            }
            Rule::Url => {
                if value.is_empty() || value.starts_with("http://") || value.starts_with("https://")
                {
                    Ok(())
                } else {
                    Err(format!("{field} must be a url"))
                }
            }
            Rule::LocalPath => {
                if value.is_empty() || crate::core::request::is_local_path(value) {
                    Ok(())
                } else {
                    Err(format!("{field} must be a local path"))
                }
            }
            Rule::Boolean => {
                if value.is_empty() || is_boolish(value) {
                    Ok(())
                } else {
                    Err(format!("{field} must be boolean"))
                }
            }
            Rule::Accepted => {
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                ) {
                    Ok(())
                } else {
                    Err(format!("{field} must be accepted"))
                }
            }
            Rule::Declined => {
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                ) {
                    Ok(())
                } else {
                    Err(format!("{field} must be declined"))
                }
            }
            Rule::Eq(expected) => {
                if value.is_empty() || value == *expected {
                    Ok(())
                } else {
                    Err(format!("{field} must be {expected}"))
                }
            }
            Rule::NotEq(denied) => {
                if value.is_empty() || value != *denied {
                    Ok(())
                } else {
                    Err(format!("{field} is invalid"))
                }
            }
            Rule::StartsWith(prefix) => {
                if value.is_empty() || value.starts_with(prefix) {
                    Ok(())
                } else {
                    Err(format!("{field} must start with {prefix}"))
                }
            }
            Rule::EndsWith(suffix) => {
                if value.is_empty() || value.ends_with(suffix) {
                    Ok(())
                } else {
                    Err(format!("{field} must end with {suffix}"))
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
                    Err(format!("{field} confirmation does not match"))
                }
            }
            Rule::Same(other_field) => {
                let other = all.get(*other_field).map(String::as_str).unwrap_or("");
                if value == other {
                    Ok(())
                } else {
                    Err(format!("{field} must match {other_field}"))
                }
            }
            Rule::In(allowed) => {
                if value.is_empty() || allowed.contains(&value) {
                    Ok(())
                } else {
                    Err(format!("{field} is invalid"))
                }
            }
            Rule::NotIn(denied) => {
                if value.is_empty() || !denied.contains(&value) {
                    Ok(())
                } else {
                    Err(format!("{field} is invalid"))
                }
            }
            Rule::Regex(pat) => {
                if value.is_empty() {
                    return Ok(());
                }
                match regex::Regex::new(pat) {
                    Ok(re) if re.is_match(value) => Ok(()),
                    Ok(_) => Err(format!("{field} format is invalid")),
                    Err(_) => Err(format!("{field} has invalid regex rule")),
                }
            }
        }
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

    #[test]
    fn local_path_rule_allows_empty_or_internal_paths() {
        let input = HashMap::new();
        assert!(Rule::LocalPath.check("redirect", "", &input).is_ok());
        assert!(
            Rule::LocalPath
                .check("redirect", "/account?tab=billing", &input)
                .is_ok()
        );
        assert!(
            Rule::LocalPath
                .check("redirect", "//example.test", &input)
                .is_err()
        );
    }
}
