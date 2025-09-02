use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub enum QuestionType {
    Single(Question),
    Grouped(QuestionHeader, Vec<QuestionType>),
}

#[derive(Deserialize, Serialize, Debug)]
pub struct QuestionHeader {
    pub raw_text: String,
    pub latex_text: String,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Question {
    pub header: QuestionHeader,
    pub raw_text: String,
    pub latex_text: String,
    pub mark_scheme: MarkScheme,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct MarkScheme {
    pub raw_text: String,
    pub latex_text: String,
}

impl QuestionType {
    pub fn into_grouped(self) -> Self {
        match self {
            QuestionType::Single(question) => {
                if question.is_empty() {
                    return Self::Grouped(question.header, Vec::new());
                }
                Self::Grouped(question.header, vec![ QuestionType::Single( Question::from(QuestionHeader::new("", ""), question.raw_text, question.latex_text, question.mark_scheme)) ])
            },
            _ => panic!("can only transform single to grouped")
        }
    }
}

impl QuestionHeader {
    pub fn new<T>(raw_text: T, latex_text: T) -> Self 
    where
        T: ToString
    {
        Self { raw_text: raw_text.to_string(), latex_text: latex_text.to_string() }
    }
}

impl Question {
    pub fn new(header: QuestionHeader) -> Self {
        Self {
            header,
            raw_text: String::new(),
            latex_text: String::new(),
            mark_scheme: MarkScheme::new(),
        }
    }
    pub fn from(header: QuestionHeader, raw_text: String, latex_text: String, mark_scheme: MarkScheme) -> Self {
        Self { header, raw_text, latex_text, mark_scheme }
    }
    pub fn from_header_and_scheme(header: QuestionHeader, mark_scheme: MarkScheme) -> Self {
        Self {
            header,
            mark_scheme,
            raw_text: String::new(),
            latex_text: String::new(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.raw_text.is_empty() && self.latex_text.is_empty() && self.mark_scheme.is_empty()
    }
}

impl MarkScheme {
    pub fn new() -> Self {
        Self {
            raw_text: String::new(),
            latex_text: String::new(),
        }
    }
    pub fn from(raw_text: String, latex_text: String) -> Self {
        Self {
            raw_text,
            latex_text,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.raw_text.is_empty() && self.latex_text.is_empty()
    }
}
