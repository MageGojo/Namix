//! 登录表单（Form Request）。

use namix::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum LoginForm {
    #[field = "username"]
    Username,
    #[field = "password"]
    Password,
    #[field = "redirect"]
    Redirect,
}

#[derive(Clone, Debug)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub redirect: String,
}

impl FormRequest for LoginRequest {
    fn redirect_to() -> FormRedirect {
        FormRedirect::Named("login")
    }

    fn from_values(req: &Request) -> Result<Self, ValidationError> {
        let v = req
            .validator()
            .rules(LoginForm::Username, &[Rule::Required, Rule::Min(3)])
            .rules(LoginForm::Password, &[Rule::Required, Rule::Min(1)])
            .rule(LoginForm::Redirect, Rule::LocalPath)
            .validate()?;

        let redirect = v.local_path_or("redirect", "/me").to_string();

        Ok(Self {
            username: v.get(LoginForm::Username).to_string(),
            password: v.get(LoginForm::Password).to_string(),
            redirect,
        })
    }
}
