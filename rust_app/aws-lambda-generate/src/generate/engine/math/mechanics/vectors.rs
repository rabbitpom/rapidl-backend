use rand::seq::SliceRandom;
use crate::generate::questionstacker::Stacker;

pub mod t1;

static GENERATORS: [fn() -> Stacker; 1] = [t1::generate];

pub fn generate() -> Stacker {
    GENERATORS.choose(&mut rand::thread_rng()).unwrap()()
}
