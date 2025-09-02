use chrono::{NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use rmp_serde::{Deserializer, Serializer};
use common_types::Generate::{GenerateId, GenerateOption};

use super::engine::{self, math, GenerateResult};
use super::question::QuestionType;

#[derive(Deserialize, Serialize)]
pub struct Paper {
    questions: Vec<QuestionType>,
    created_by: i64,
    created_on: NaiveDateTime,
    generated_catagory: GenerateId,
    generated_options: Vec<GenerateOption>,
}
impl Paper {
    pub fn new(created_by: i64, generated_catagory: GenerateId, generated_options: Vec<GenerateOption>) -> Self {
        Self {
            created_by,
            generated_catagory,
            generated_options,
            created_on: Utc::now().naive_utc(),
            questions: Vec::new(),
        }
    }
    pub fn populate(&mut self) -> GenerateResult<()> {
        match self.generated_catagory {
            GenerateId::MathsCore => {
                self.questions = math::pure::generate_from_options(engine::GENERATE_QUESTIONS_PER_TOPIC, &self.generated_options)?;
            },
            GenerateId::MathsMechanics => {
                self.questions = math::mechanics::generate_from_options(engine::GENERATE_QUESTIONS_PER_TOPIC, &self.generated_options)?;
            },
            GenerateId::MathsStatistics => {
                self.questions = math::statistics::generate_from_options(engine::GENERATE_QUESTIONS_PER_TOPIC, &self.generated_options)?;
            },
            _ => ()
        }
        Ok(())
    }
}
