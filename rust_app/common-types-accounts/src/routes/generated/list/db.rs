use diesel::QueryableByName;
use db_schema::{generation, hooked_sql_types::GenerationStatus};
use serde::{Deserialize, Serialize};
use chrono::NaiveDateTime;
use uuid::Uuid;
use garde::Validate;

#[derive(Deserialize, Validate)]
pub struct Pagination {
    #[garde(skip)]
    pub page: usize,
    #[garde(custom(is_valid_page_size))]
    pub page_size: usize,
    #[garde(skip)]
    pub get_total_pages: bool,
}

#[derive(QueryableByName, PartialEq, Debug, Serialize)]
#[diesel(table_name = generation)]
pub struct GenerationQueryable {
    pub status: GenerationStatus,
    pub createdat: NaiveDateTime,
    pub finishedon: Option<NaiveDateTime>,
    #[serde(serialize_with = "uuid::serde::simple::serialize")]
    pub jobid: Uuid,
    pub creditsused: i16,
    pub category: String,
    pub options: String,
    pub displayname: String,
}

pub fn is_valid_page_size(value:&usize, _: &()) -> garde::Result {
    if value != &5 && value != &10 {
        return Err(garde::Error::new("can only be 5 or 10"));
    }
    Ok(())
}
