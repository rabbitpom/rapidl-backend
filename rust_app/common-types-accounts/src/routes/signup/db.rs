use serde::Deserialize;
use garde::Validate;
use diesel::prelude::*;
use crate::Schema::users;

#[derive(Insertable)]
#[diesel(table_name = users)]
#[allow(non_snake_case)]
pub struct User<'a> {
    pub username: &'a str,
    pub email: &'a str,
    pub emailverified: bool,
    pub bcryptpass: &'a [u8],
}

#[derive(Deserialize, Debug, Validate)]
pub struct RequestPayload {
    #[garde(ascii, alphanumeric, length(min=3, max=16))]
    pub username: String,
    #[garde(email, length(max=320))]
    pub email: String,
    #[garde(ascii, pattern(r#"^[^\s]+$"#), length(min=8, max=16))]
    pub password: String,
}
