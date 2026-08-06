//! 注册表单（Form Request）：进控制器即合法字段。

use namix::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum RegisterForm {
    #[field = "username"]
    Username,
    #[field = "password"]
    Password,
    #[field = "password_confirmation"]
    PasswordConfirmation,
}

/// 校验通过后的注册输入。
#[derive(Clone, Debug)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
}

impl FormRequest for RegisterRequest {
    fn redirect_to() -> FormRedirect {
        FormRedirect::Named("register")
    }

    fn from_values(req: &Request) -> Result<Self, ValidationError> {
        let v = req
            .validator()
            .rules(
                RegisterForm::Username,
                &[
                    Rule::Required,
                    Rule::Between(3, 16),
                    Rule::Regex(r"^[a-zA-Z0-9_]+$"),
                ],
            )
            .rules(
                RegisterForm::Password,
                &[Rule::Required, Rule::Min(8), Rule::Confirmed],
            )
            .custom(RegisterForm::Password, |password, _| {
                let has_upper = password.chars().any(|c| c.is_ascii_uppercase());
                let has_lower = password.chars().any(|c| c.is_ascii_lowercase());
                let has_digit = password.chars().any(|c| c.is_ascii_digit());
                let has_special = password.chars().any(|c| !c.is_ascii_alphanumeric());
                if has_upper && has_lower && has_digit && has_special {
                    Ok(())
                } else {
                    Err("password must include upper, lower, digit and special char".into())
                }
            })
            .validate()?;

        Ok(Self {
            username: v.get(RegisterForm::Username).to_string(),
            password: v.get(RegisterForm::Password).to_string(),
        })
    }
}
