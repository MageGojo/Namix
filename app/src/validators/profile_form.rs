//! 个人资料表单（Form Request）。

use namix::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum ProfileForm {
    #[field = "display_name"]
    DisplayName,
    #[field = "email"]
    Email,
    #[field = "bio"]
    Bio,
}

#[derive(Clone, Debug)]
pub struct ProfileRequest {
    pub display_name: String,
    pub email: String,
    pub bio: String,
}

impl FormRequest for ProfileRequest {
    fn redirect_to() -> FormRedirect {
        FormRedirect::Named("me")
    }

    fn from_values(req: &Request) -> Result<Self, ValidationError> {
        let v = req
            .validator()
            .rules(
                ProfileForm::DisplayName,
                &[Rule::Required, Rule::Between(1, 32)],
            )
            .rules(ProfileForm::Email, &[Rule::Max(64)])
            .rules(ProfileForm::Bio, &[Rule::Max(500)])
            .validate()?;

        Ok(Self {
            display_name: v.get(ProfileForm::DisplayName).to_string(),
            email: v.get(ProfileForm::Email).to_string(),
            bio: v.get(ProfileForm::Bio).to_string(),
        })
    }
}
