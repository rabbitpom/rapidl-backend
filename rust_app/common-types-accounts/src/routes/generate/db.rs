use diesel::prelude::*;
use chrono::NaiveDateTime;
use serde::Deserialize;
use garde::Validate;
use crate::common_types::Generate::{GenerateId, GenerateOption::{self, *}};
use crate::Schema::{generation, hooked_sql_types::GenerationStatus};

#[derive(Deserialize, Validate)]
#[garde(context(RequestPayload))]
pub struct RequestPayload {
    #[garde(custom(check_generate_options))]
    pub choices: Vec<GenerateOption>,
    #[garde(skip)]
    pub payload_id: GenerateId,
}

fn check_generate_options(value: &Vec<GenerateOption>, context: &RequestPayload) -> garde::Result {
    match context.payload_id {
        GenerateId::MathsMechanics => {
            for generate_option in value.into_iter() {
                match generate_option {
                    SUVAT | Momentum | Graphs | Moments | Pullies | InclinedSlopes | Projectiles | Vectors => (),
                    _ => return Err(garde::Error::new("invalid options")),
                }
            }
            Ok(())
        },
        GenerateId::MathsStatistics => {
            for generate_option in value.into_iter() {
                match generate_option {
                    Probability | Graphs | HypothesisTesting | NormalDistribution | BinomialDistribution => (),
                    _ => return Err(garde::Error::new("invalid options")),
                }
            }
            Ok(())
        },
        GenerateId::MathsCore => {
            for generate_option in value.into_iter() {
                match generate_option {
                    Graphs | Algebra | Integration | Differentiation | TrigonometricIdentities | CoordinateGeometry | SequencesAndSeries => (),
                    _ => return Err(garde::Error::new("invalid options")),
                }
            }
            Ok(())
        },

        _ => Err(garde::Error::new("bad data")),
    }
}

#[derive(Insertable)]
#[diesel(table_name = generation)]
#[allow(non_snake_case)]
pub struct InsertableGeneration {
    pub userid: i64,
    pub status: GenerationStatus,
    pub createdat: NaiveDateTime,
    pub jobid: uuid::Uuid,
    pub creditsused: i16,
    pub category: String,
    pub options: String,
    pub displayname: String,
}
