//! Input types of the builders. They mirror the main backend's `StudyQuestion` / `Solution` (spec 4.5).
use crate::prompt::Lang;

/// Question kind (`single`, `multi`, `hotspot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QKind {
    Single,
    Multi,
    Hotspot,
}

/// One option of a single / multiple choice question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionText {
    /// The option letter, `A`..`H`.
    pub key: String,
    pub text: String,
}

/// One statement of a hotspot question with its choices (for example `Yes` / `No`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotspotItem {
    pub text: String,
    pub choices: Vec<String>,
}

/// A question as shown to the user: no answer key and no explanation (that is [`Solution`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudyQuestion {
    pub id: String,
    pub kind: QKind,
    /// The language the texts are actually in (after the backend's fallback).
    pub lang: Lang,
    pub stem: String,
    pub options: Vec<OptionText>,
    pub items: Vec<HotspotItem>,
    pub active: bool,
    pub cut: bool,
}

/// The stored answer key: letters for single / multi, one choice index per item for hotspot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAnswer {
    Letters(Vec<String>),
    Indexes(Vec<usize>),
}

/// The question bank's solution data. Only explanation prompts and summaries take it; hints never do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    pub answer: KeyAnswer,
    pub explanation: String,
    pub mnemonic: String,
    pub note: String,
    /// `None` = old-layout compatibility text.
    pub explain_lang: Option<Lang>,
    /// Texts of the correct option(s) / choice(s) in every language the question has (full caps only).
    pub key_texts: Vec<String>,
}

/// One entry of a stored user answer (`[str|n|null]`): a letter, a choice index, or not answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserEntry {
    Text(String),
    Index(usize),
    Null,
}
