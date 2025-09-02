use diesel::{Selectable, Queryable};
use db_schema::{generation, hooked_sql_types::GenerationStatus};
use serde::{Deserialize, Deserializer};
use chrono::NaiveDateTime;
use garde::Validate;

#[derive(Deserialize, Validate)]
pub struct GenerationQuery {
    #[garde(ascii)]
    pub id: String,
}

#[derive(Deserialize, Validate)]
pub struct GenerationNameChangeQuery {
    #[garde(ascii)]
    pub id: String,
    #[garde(ascii, length(min=0, max=20))]
    pub displayname: String,
}

fn deserialize_by_comma<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    Ok(s.split(',').map(|part| part.trim().to_string()).collect())
}

#[derive(Deserialize, Validate)]
pub struct GenerationBatchQuery {
    #[serde(deserialize_with = "deserialize_by_comma")]
    #[garde(inner(ascii))]
    pub ids: Vec<String>,
}

#[derive(Queryable, Selectable, PartialEq, Debug)]
#[diesel(table_name = generation)]
pub struct GenerationSelectable {
    pub status: GenerationStatus,
    pub createdat: NaiveDateTime,
    pub finishedon: Option<NaiveDateTime>,
    pub displayname: String,
    pub options: String,
    pub category: String,
    pub creditsused: i16,
}

#[derive(Queryable, Selectable, PartialEq, Debug)]
#[diesel(table_name = generation)]
pub struct GenerationSelectableWithJobId {
    pub jobid: uuid::Uuid,
    pub status: GenerationStatus,
    pub createdat: NaiveDateTime,
    pub finishedon: Option<NaiveDateTime>,
    pub displayname: String,
    pub options: String,
    pub category: String,
    pub creditsused: i16,
}
