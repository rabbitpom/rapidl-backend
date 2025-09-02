use serde::Deserialize;
use garde::Validate;
use diesel::prelude::*;
use crate::Schema::allocatedcredits;
use chrono::NaiveDateTime;

#[derive(Insertable)]
#[diesel(table_name = allocatedcredits)]
#[allow(non_snake_case)]
pub struct InsertableAllocatedCredits {
    pub credits: i32,
    pub expireat: NaiveDateTime,
    pub userid: i64,
}

#[derive(Deserialize, Debug, Validate)]
pub struct RequestQuery {
    #[garde(ascii)]
    pub token: String,
}
