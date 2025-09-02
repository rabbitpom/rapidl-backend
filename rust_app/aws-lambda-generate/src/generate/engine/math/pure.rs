use common_types::Generate::GenerateOption;
use crate::generate::{
    engine::{
        GenerateResult,
        GenerateFailure,
    },
    question::QuestionType,
};

pub fn get_generator_from_option(option: &GenerateOption) -> Option<fn() -> QuestionType> {
    unimplemented!()
}

pub fn generate_from_options(target_amount_per_option: usize, options: &Vec<GenerateOption>) -> GenerateResult<Vec<QuestionType>> {
    let mut questions = Vec::new();

    for generate_option in options.iter() {
        let pointer = get_generator_from_option(generate_option).ok_or(GenerateFailure::InvalidOption( generate_option.clone() ))?;
        for _ in 0..target_amount_per_option {
            let question = pointer();
            questions.push(question);
        }
    }

    Ok(questions)
}
