//! The AI steps of a file import (spec IA-1 … IA-7): lines and windows, classification, segmentation, conversion of
//! questions as they are, and questions made from study material. Pure: no I/O, no clock, randomness from the job seed.
//! Every piece of file text that enters a prompt goes through `gen::prompt::clean` and sits only inside a nonce fence.
pub mod classify;
pub mod convert;
pub mod count;
pub mod job;
pub mod lines;
pub mod material;
pub mod segment;
pub mod whole;

/// What a recognised file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Questions,
    Material,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Questions => "questions",
            Mode::Material => "material",
        }
    }
}

/// Characters of the main part of a window (IA-1).
pub const WINDOW_MAIN_CHARS: usize = 10_000;
/// The overlap after the main part: the longest question allowed, so a question that starts in the main part always ends
/// inside its window.
pub const WINDOW_OVERLAP_CHARS: usize = 3_000;
/// More windows than this refuse the questions mode (`ai.import.tooMuchText`, 300 000 characters).
pub const MAX_WINDOWS: usize = 30;
/// A line longer than this is cut (IA-1).
pub const LINE_MAX_CHARS: usize = 1_000;
/// A question's source text may not be longer than this (`ai.import.tooLongQuestion`).
pub const MAX_QUESTION_CHARS: usize = 3_000;
/// The questions-mode text limit shown in `errorArg` of `tooMuchText`.
pub const QUESTIONS_TEXT_MAX: usize = MAX_WINDOWS * WINDOW_MAIN_CHARS;

/// The JSON object of a model answer: an optional code fence is removed, then the text from the first `{` to the last
/// `}` is parsed (GP-3's leniency). `None` when there is none.
pub fn json_object(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    let unfenced: String;
    let mut body = trimmed;
    if trimmed.starts_with("```") {
        let mut lines: Vec<&str> = trimmed.lines().collect();
        lines.remove(0);
        if lines.last().is_some_and(|l| l.trim() == "```") {
            lines.pop();
        }
        unfenced = lines.join("\n");
        body = &unfenced;
    }
    let start = body.find('{')?;
    let end = body.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&body[start..=end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_objects_are_found_around_fences_and_prose() {
        assert!(json_object("{\"a\":1}").is_some());
        assert!(json_object("```json\n{\"a\":1}\n```").is_some());
        assert!(json_object("Sure! {\"a\":1} done").is_some());
        assert!(json_object("no braces").is_none());
        assert!(json_object("} {").is_none());
        assert!(json_object("{broken").is_none());
    }

    #[test]
    fn mode_words() {
        assert_eq!(Mode::Questions.as_str(), "questions");
        assert_eq!(Mode::Material.as_str(), "material");
        assert_eq!(QUESTIONS_TEXT_MAX, 300_000);
    }
}
