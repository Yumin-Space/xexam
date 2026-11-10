//! Data models. **Field declaration order = key order of the legacy JSON** (serde_json has preserve_order on, so serialization emits keys in this order),
//! do not reorder; the frontend and the golden responses depend on it.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Option (single / multi-choice question): `{"key": "A", "text": "…"}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opt {
    pub key: String,
    pub text: String,
}

/// One sub-item of a hotspot question: `{"text": "…", "choices": ["はい", "いいえ"]}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub text: String,
    pub choices: Vec<String>,
}

/// Structure split out of the Chinese translation. Single / multi: `{"stem", "options"}`; hotspot: `{"stem", "items"}`.
/// The two shapes used different keys in the legacy version, so there are two optional fields and None is skipped on serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Zh {
    pub stem: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub options: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub items: Option<Vec<String>>,
    /// Hotspot: the options written in **this language** at the end of each sub-item (`是 / 否`, `Yes / No`, ...); present only if the count matches the Japanese one.
    /// When absent (old file, or this column has no options written) the UI falls back to showing the Japanese options.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub choices: Option<Vec<Vec<String>>>,
}

/// Structure split out of the English version, exactly the same as the Chinese one (see [`Zh`]).
/// Present only when this question's English lines up one-to-one with the Japanese (same option letters, same number of sub-items); otherwise there is no `en` and the page greys out the English button.
pub type En = Zh;

/// Question kind. Serialized as `"single" | "multi" | "hotspot"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Single,
    Multi,
    Hotspot,
}

/// Answer: for single / multi it is letters (`["B"]`); for hotspot it is the index chosen for each sub-item (`[0, 1, 0]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Answer {
    Letters(Vec<String>),
    Indexes(Vec<i64>),
}

/// One question produced by the parser (what is stored in `questions.data` is the part left after removing id / sheet / row).
///
/// Key order on the outside (`/api/bank`): kind, tag, stem, options, items, answer, translation, zh, [english, en,]
/// explanation, mnemonic, note, id, sheet, row (the two in brackets appear only when there is English) -- the same as the key order of `questions.data` plus id, sheet, row appended at the end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub kind: Kind,
    pub tag: String,
    pub stem: String,
    pub options: Vec<Opt>,
    pub items: Vec<Item>,
    pub answer: Answer,
    pub translation: String,
    pub zh: Option<Zh>,
    /// Original English text (column A of the 4-column format). Empty when there is no English, and then it does not appear in the JSON, so the output for old questions is exactly the same as before.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub english: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub en: Option<En>,
    pub explanation: String,
    pub mnemonic: String,
    pub note: String,
    // -- The following three exist only in memory / in the outgoing output; they are not written into data when stored --
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sheet: String,
    #[serde(default)]
    pub row: i64,
}

/// Statistics for one sheet at parse time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedRow {
    pub row: i64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedSheet {
    pub name: String,
    pub total: i64,
    pub usable: i64,
    pub skipped: Vec<SkippedRow>,
}

/// Parse result for the whole workbook.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedWorkbook {
    pub sheets: Vec<ParsedSheet>,
    pub questions: Vec<Question>,
}

/// One import record (a row of the `imports` table, also an element of `/api/imports`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    pub id: i64,
    pub at: String,
    pub filename: String,
    pub added: i64,
    pub updated: i64,
    pub removed: i64,
    pub restored: i64,
    pub skipped: i64,
    pub total: i64,
}

/// The request after `/api/attempt` has been sanitized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptResult {
    pub id: String,
    pub correct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub results: Vec<AttemptResult>,
    pub sheets: Vec<String>,
    pub mode: String,
    pub seconds: i64,
}

/// The "per-sheet" block in preview / confirm results. For the key order see legacy golden responses #114 / #120.
pub type JsonValue = Value;

/// The cleaned per-question answers of an `/api/attempt` request, index-aligned with `Attempt::results`
/// (`None` = not answered / not sent / malformed). Each `Some` is a JSON array as submitted (strings, integers, nulls).
pub type AttemptAnswers = Vec<Option<Value>>;

/// Which question bank an upload goes to (header `X-Bank-Target`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportTarget {
    /// No header: the v0.4 behaviour against the legacy bank (created at confirm if it does not exist).
    Legacy,
    /// `new`: a new bank is created at confirm.
    New,
    /// A bank id: update (replace) that bank.
    Bank(i64),
}

impl ImportTarget {
    /// Whether the upload named its target explicitly (only then does the confirm response carry `bankId`).
    pub fn is_explicit(&self) -> bool {
        !matches!(self, ImportTarget::Legacy)
    }
}

/// What `delete_bank` removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeletedBank {
    pub id: i64,
    /// The deleted bank was the legacy bank (the target of uploads without `X-Bank-Target`).
    pub legacy: bool,
}

/// Settings stored in the `settings` table: key, allowed values, default. The JSON key order of `/api/settings` is this order.
pub const SETTING_THEME: &str = "theme";
pub const SETTING_UI_LANG: &str = "uiLang";
pub const SETTING_AUTO_ADVANCE: &str = "autoAdvance";
pub const SETTING_QUESTION_LANG: &str = "questionLang";
pub const SETTING_KEYS: [&str; 4] = [
    SETTING_THEME,
    SETTING_UI_LANG,
    SETTING_AUTO_ADVANCE,
    SETTING_QUESTION_LANG,
];
/// Keys removed by `/api/settings/reset` (`questionLang` is kept).
pub const RESETTABLE_SETTINGS: [&str; 3] = [SETTING_THEME, SETTING_UI_LANG, SETTING_AUTO_ADVANCE];

/// Default value of a known setting key (`None` for unknown keys).
pub fn setting_default(key: &str) -> Option<Value> {
    Some(match key {
        SETTING_THEME => Value::from("system"),
        SETTING_UI_LANG => Value::from("en"),
        SETTING_AUTO_ADVANCE => Value::Bool(false),
        SETTING_QUESTION_LANG => Value::from("ja"),
        _ => return None,
    })
}

/// Whether `value` is allowed for the known setting `key` (unknown keys are never valid).
pub fn setting_valid(key: &str, value: &Value) -> bool {
    let one_of = |allowed: &[&str]| value.as_str().is_some_and(|s| allowed.contains(&s));
    match key {
        SETTING_THEME => one_of(&["system", "light", "dark"]),
        SETTING_UI_LANG => one_of(&["en", "zh", "ja"]),
        SETTING_AUTO_ADVANCE => value.is_boolean(),
        SETTING_QUESTION_LANG => one_of(&["ja", "zh", "en"]),
        _ => false,
    }
}
