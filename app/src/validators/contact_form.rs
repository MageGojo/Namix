//! ContactForm — 基础表单验证器。

use namix::prelude::*;

#[derive(Clone, Copy, Debug, FormField)]
pub enum ContactForm {
    #[field = "contact"]
    Contact,
}

pub fn validate(req: &Request) -> Result<Validated, ValidationError> {
    req.validator()
        .rules(
            ContactForm::Contact,
            &[Rule::Required, Rule::Between(1, 64)],
        )
        .validate()
}
