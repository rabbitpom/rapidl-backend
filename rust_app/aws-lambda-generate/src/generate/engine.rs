use common_types::Generate::GenerateOption;

pub mod math;

pub type GenerateResult<T> = Result<T, GenerateFailure>;

#[derive(Debug)]
pub enum GenerateFailure {
    InvalidOption( GenerateOption ),
}

pub const GENERATE_QUESTIONS_PER_TOPIC: usize = 3;
