use serde::Deserialize;
use garde::Validate;

#[derive(Deserialize, Debug, Validate)]
pub struct RequestPayload {
    #[garde(email, length(max=320))]
    pub email: String,
}
