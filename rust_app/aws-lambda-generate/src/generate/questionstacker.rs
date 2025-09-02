use super::question::{Question, QuestionType};

#[derive(Debug)]
pub struct Stacker {
    questions: Vec<QuestionType>,
}
impl Stacker {
    pub fn new() -> Self {
        Self {
            questions: Vec::new(),
        }
    }
    pub fn next_root_question(&mut self, question: Question) {
        let question = QuestionType::Single(question);
        self.questions.push(question);
    }
    pub fn next_root_sub_question(&mut self, question: Question) {
        let questions_len = self.questions.len();
        let root_question = &mut self.questions[questions_len - 1];
        let QuestionType::Grouped(_, ref mut questions) = root_question else {
            let root_question = self.questions.pop().unwrap();
            let grouped_root_question = root_question.into_grouped();
            self.questions.push(grouped_root_question);
            return self.next_root_sub_question( question )
        };
        questions.push( QuestionType::Single(question) )
    }
    pub fn next_depth_sub_question(&mut self, question: Question) {
        let sub_questions = recursive_last_morph(&mut self.questions);
        sub_questions.push( QuestionType::Single(question) )
    }
    pub fn consume_get_questions(self) -> Vec<QuestionType> {
        self.questions
    }
}

fn recursive_last_morph(questions: &mut Vec<QuestionType>) -> &mut Vec<QuestionType> {
    if questions.is_empty() || questions.len() == 1 {
        return questions;
    }
    let last_index = questions.len() - 1;
    match &questions[last_index] {
        QuestionType::Grouped(_, _) => {
            // this fixes the borrow issues
            if let QuestionType::Grouped(_, ref mut q) = &mut questions[last_index] {
                 return recursive_last_morph(q);
            } else {
                unreachable!()
            }
        },
        _ => {
            let root_question = questions.pop().unwrap();
            let grouped_root_question = root_question.into_grouped();
            questions.push(grouped_root_question);
            return recursive_last_morph(questions);
        },
    }
}
