use common_types::Generate::GenerateOption;
use crate::generate::{
    engine::{
        GenerateResult,
        GenerateFailure,
    },
    question::QuestionType,
    questionstacker::Stacker,
};

mod graphs;
mod inclinedslopes;
mod moments;
mod momentum;
mod projectiles;
mod pullies;
mod suvat;
mod vectors;

pub fn get_generator_from_option(option: &GenerateOption) -> Option<fn() -> Stacker> {
    match option {
        GenerateOption::SUVAT => Some(suvat::generate),
        GenerateOption::Vectors => Some(vectors::generate),
        _=> None,
    }
}

pub fn generate_from_options(target_amount_per_option: usize, options: &Vec<GenerateOption>) -> GenerateResult<Vec<QuestionType>> {
    let mut questions = Vec::new();

    for generate_option in options.iter() {
        let pointer = get_generator_from_option(generate_option).ok_or(GenerateFailure::InvalidOption( generate_option.clone() ))?;
        for _ in 0..target_amount_per_option {
            let questionstacker = pointer();
            let mut generated_questions = questionstacker.consume_get_questions();
            questions.append(&mut generated_questions);
        }
    }

    Ok(questions)
}
