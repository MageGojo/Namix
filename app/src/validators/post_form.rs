//! 发帖表单（Form Request）。

use crate::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum PostForm {
    #[field = "title"]
    Title,
    #[field = "body"]
    Body,
}

#[derive(Clone, Debug)]
pub struct PostRequest {
    pub title: String,
    pub body: String,
}

impl FormRequest for PostRequest {
    fn redirect_to() -> FormRedirect {
        FormRedirect::named(AppRoute::Posts)
    }

    fn from_values(req: &Request) -> Result<Self, ValidationError> {
        let v = req
            .validator()
            .rules(PostForm::Title, &[Rule::Required, Rule::Between(1, 80)])
            .rules(PostForm::Body, &[Rule::Required, Rule::Between(1, 2000)])
            .validate()?;

        Ok(Self {
            title: v.get(PostForm::Title).to_string(),
            body: v.get(PostForm::Body).to_string(),
        })
    }
}
