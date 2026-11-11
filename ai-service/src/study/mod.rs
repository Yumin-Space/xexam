//! AI+ study features (v0.9): hint, explanation and summary prompts, the hint guard, the chat titles, the strict wire
//! types of the backend's data routes and the record-identity cache. Everything here is pure (no I/O) except the cache,
//! which only holds memory.
pub mod cache;
pub mod explain;
pub mod guard;
pub mod hint;
pub mod model;
pub mod render;
pub mod summary;
pub mod title;
pub mod wire;

use crate::prompt::Lang;

/// At most this many wrong answers go into one summary (owner's plan).
pub const MAX_SUMMARY_ITEMS: usize = 70;
/// The stored user message of a hint / explanation keeps at most this many characters.
pub const USER_MESSAGE_CHARS: usize = 4_000;

/// The reply-language rule of every study prompt. It starts with the same words as the AI Chat line
/// (`Always reply in {language} (`), but has no "unless the user asks" tail: a study prompt has no free-text user turn,
/// only data, and the tail would let text inside a question steer the language.
pub fn language_line(lang: Lang) -> String {
    format!(
        "Always reply in {} (the language chosen in the user's Xexam settings).",
        lang.name()
    )
}

/// The language code stored with a message and sent to the browser: `en`, `zh-CN` or `ja`.
pub fn lang_code(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "en",
        Lang::Zh => "zh-CN",
        Lang::Ja => "ja",
    }
}

pub use explain::{explain_prompt, practice_explain_prompt, result_explain_prompt, Mine};
pub use hint::{hint_prompt, study_user_message};
pub use model::{HotspotItem, KeyAnswer, OptionText, QKind, Solution, StudyQuestion, UserEntry};
pub use summary::{
    summary_prompt, summary_user_message, BasedOn, RecordInfo, RecordKind, Sample, SummaryItem,
};
pub use title::{at_digits_to_at, clamp_started_at, suffix_key, summary_ref, ChatKind};
