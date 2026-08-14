//! 个人资料表单（Form Request）。

use crate::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum ProfileForm {
    #[field = "display_name"]
    DisplayName,
    #[field = "email"]
    Email,
    #[field = "bio"]
    Bio,
    #[field = "avatar"]
    Avatar,
}

#[derive(Clone, Debug)]
pub struct ProfileRequest {
    pub display_name: String,
    pub email: String,
    pub bio: String,
    pub avatar: Option<namix::UploadedFile>,
}

impl FormRequest for ProfileRequest {
    fn redirect_to() -> FormRedirect {
        FormRedirect::named(AppRoute::Me)
    }

    fn from_values(req: &Request) -> Result<Self, ValidationError> {
        let except = req
            .get::<crate::services::session::LoginUser>()
            .map(|user| user.id.to_string())
            .unwrap_or_default();
        let v = req
            .validator()
            .rules(
                ProfileForm::DisplayName,
                &[Rule::Required, Rule::Between(1, 32)],
            )
            .rules(
                ProfileForm::Email,
                &[
                    Rule::Email,
                    Rule::Max(64),
                    Rule::unique_ignore_col("profiles", "email", "user_id", except),
                ],
            )
            .rules(ProfileForm::Bio, &[Rule::Max(500)])
            .rules(
                ProfileForm::Avatar,
                &[
                    Rule::Image,
                    Rule::Mimes(&["png", "jpg", "jpeg", "webp", "gif"]),
                    Rule::MaxBytes(2_000_000),
                ],
            )
            .validate()?;

        Ok(Self {
            display_name: v.get(ProfileForm::DisplayName).to_string(),
            email: v.get(ProfileForm::Email).to_string(),
            bio: v.get(ProfileForm::Bio).to_string(),
            avatar: v.file_field(ProfileForm::Avatar).cloned(),
        })
    }
}
