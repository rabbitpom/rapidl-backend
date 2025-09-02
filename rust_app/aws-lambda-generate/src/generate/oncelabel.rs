use rand::Rng;

use super::formatter;

pub struct OnceLabel {
    free_labels: Vec<&'static str>,
    free_raw_labels: Vec<&'static str>,
    free_symbols: Vec<&'static str>,
}
impl OnceLabel {
    pub fn new() -> Self {
        Self {
            free_labels: Vec::from(formatter::LABELLED_IDENTIFIERS),
            free_raw_labels: Vec::from(formatter::LABELLED_IDENTIFIERS_RAW),
            free_symbols: Vec::from(formatter::LABELLED_SYMBOLS),
        }
    }
    pub fn next_symbol_raw(&mut self) -> &'static str {
        let index = rand::thread_rng().gen_range(0..self.free_symbols.len());
        self.free_symbols.swap_remove(index)
    }
    pub fn next_label_raw(&mut self) -> (&'static str, &'static str) {
        let index = rand::thread_rng().gen_range(0..self.free_labels.len());
        (self.free_labels.swap_remove(index), self.free_raw_labels.swap_remove(index))
    }
}
