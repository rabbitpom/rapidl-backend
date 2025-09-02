use serde::Deserialize;
use garde::Validate;

#[derive(Deserialize, Debug, Validate)]
pub struct RequestPayload {
    #[garde(email, length(max=320))]
    pub email: String,
    #[garde(ascii, pattern(r#"^[^\s]+$"#), length(min=8, max=16))]
    pub password: String,
}
