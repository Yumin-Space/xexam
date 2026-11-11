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

/// The explanation, memory aid and answer note written in one language's answer column of the six-column layout
/// (D English, E Chinese, F Japanese): `{"explanation": "…", "mnemonic": "…", "note": "…"}`. All three keys are always
/// present (an empty string when that column does not have it), so the frontend never has to test for a missing key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ExplainText {
    pub explanation: String,
    pub mnemonic: String,
    /// The note written in parentheses after the answer ("C, D (confidence: high)" -> "confidence: high"), several joined with "；"
    pub note: String,
}

impl ExplainText {
    pub fn is_empty(&self) -> bool {
        self.explanation.is_empty() && self.mnemonic.is_empty() && self.note.is_empty()
    }
}

/// Per-language explanations of a six-column question: `{"en": {...}, "zh": {...}, "ja": {...}}`. A language whose
/// answer column has no explanation, memory aid or note is left out (the frontend then shows another language's text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Explain {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub en: Option<ExplainText>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub zh: Option<ExplainText>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ja: Option<ExplainText>,
}

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
/// [lang, ja,] explanation, mnemonic, note, [explain,] id, sheet, row -- the same as the key order of `questions.data` plus
/// id, sheet, row appended at the end. Keys in brackets appear only when they have a value: `english` / `en` for the
/// four-column layout, `lang` / `ja` / `explain` for the six-column layout (whose base text is English, so it never has
/// `english` / `en`); a 3-column question has none of them.
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
    /// Language of the base text (`stem`, `options`, `items` and the hotspot `choices`): `"en"` for the six-column layout
    /// (English is authoritative there). Empty for the 3 / 4-column layouts, whose base text is Japanese; it is then left
    /// out of the JSON, so their output stays byte-identical to earlier versions.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub lang: String,
    /// Six-column layout only: the Japanese question (column C) split along the English structure, same shape as `zh`.
    /// Absent when column C does not line up with the English question (the page then greys out the Japanese button).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ja: Option<Zh>,
    /// Compatibility text for clients that know only one explanation. Six-column questions: the Chinese text when column E
    /// has one, else the Japanese (F), else the English (D); the same rule applies to `mnemonic` and `note`.
    pub explanation: String,
    pub mnemonic: String,
    pub note: String,
    /// Six-column layout only: the explanation / memory aid / note of each language (see [`Explain`]).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub explain: Option<Explain>,
    // -- The following three exist only in memory / in the outgoing output; they are not written into data when stored --
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub sheet: String,
    #[serde(default)]
    pub row: i64,
}

/// The value of [`Question::lang`] for the six-column layout (English base text).
pub const SIX_COLUMN_LANG: &str = "en";

impl Question {
    /// Whether the question comes from the six-column layout (its base text is English: `lang` is `"en"`). Since v0.7 only
    /// that layout can be imported, and the questions of the 3- and 4-column layouts (Japanese base text, no `lang`) are
    /// retired (see `Store::retire_old_layout_questions`).
    pub fn is_six_column(&self) -> bool {
        self.lang == SIX_COLUMN_LANG
    }
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
    /// Rows that were imported but deserve a look (six-column layout: the Chinese or Japanese answer differs from the
    /// English one). Same shape as `skipped`; left out of the JSON when empty, so older outputs stay byte-identical.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<SkippedRow>,
}

/// Parse result for the whole workbook.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedWorkbook {
    pub sheets: Vec<ParsedSheet>,
    pub questions: Vec<Question>,
}

impl ParsedWorkbook {
    /// The parser read at least one question in the old 3- or 4-column layout (a question that is not six-column, see
    /// [`Question::is_six_column`]). Such a workbook is refused by every import entry point since v0.7.
    pub fn has_old_layout(&self) -> bool {
        self.questions.iter().any(|q| !q.is_six_column())
    }
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
    /// v0.6.5: an exam or a practice session (`"kind"` of the request; stored in `attempts.kind`)
    pub kind: AttemptKind,
}

/// What kind of study session a record is (`attempts.kind`, schema v4). Both kinds update the wrong-answer notebook the same
/// way and count toward the same retention limit; they differ only in how the page shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttemptKind {
    /// A timed exam (the only kind before v0.6.5; the column default)
    #[default]
    Exam,
    /// A practice session with immediate feedback
    Practice,
}

impl AttemptKind {
    /// The stored and serialised name: `"exam"` or `"practice"`.
    pub fn as_str(self) -> &'static str {
        match self {
            AttemptKind::Exam => "exam",
            AttemptKind::Practice => "practice",
        }
    }
    pub fn parse(s: &str) -> Option<AttemptKind> {
        match s {
            "exam" => Some(AttemptKind::Exam),
            "practice" => Some(AttemptKind::Practice),
            _ => None,
        }
    }
}

/// The practice / exam session a study record is saved from (v0.9): the random key (32 lower-case hex digits) the browser made
/// when the session started and the start time (unix seconds, the browser's clock). `POST /api/attempt` carries both as the
/// optional `"key"` / `"startedAt"`; an invalid one is stored as NULL, never refused. The AI service names the one chat of a
/// session by the key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionTag {
    pub key: Option<String>,
    pub started_at: Option<i64>,
}

/// The existing question banks a plain user is given (schema v4: their `user_banks` rows), as `"banks"` of
/// `POST /api/users/create` / `update` asks for them. Independent of the user's "include banks imported later" flag
/// (`users.new_banks`), which only adds a row for each bank created later. Administrators always see every bank, whatever is
/// stored for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankAccess {
    /// Every bank that exists when the change is written (the `"all"` shorthand of the API); not the banks created later
    All,
    /// Only these banks (sorted, without duplicates; may be empty: the user sees no bank)
    Only(Vec<i64>),
}

/// The banks a request may read: everything (administrators), or a set of bank ids (a plain user's `user_banks` rows plus,
/// while they manage banks, the banks they own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BankScope {
    All,
    Only(std::collections::BTreeSet<i64>),
}

impl BankScope {
    pub fn is_all(&self) -> bool {
        matches!(self, BankScope::All)
    }
    /// Whether a question of bank `bank` (`None`: a question without a bank, written by an older binary) is visible.
    pub fn allows(&self, bank: Option<i64>) -> bool {
        match self {
            BankScope::All => true,
            BankScope::Only(set) => bank.is_some_and(|b| set.contains(&b)),
        }
    }
}

/// Who is asking, as far as question banks are concerned. Built per request from the signed-in user (never from the request
/// body): `admin` = role administrator; `can_manage` = may import banks and update / delete the banks they own (always true for
/// administrators, who manage every bank); `scope` = the banks they may see in exams, records and the notebook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Viewer {
    pub user: i64,
    pub admin: bool,
    pub can_manage: bool,
    pub scope: BankScope,
}

impl Viewer {
    /// An administrator: sees and manages every bank.
    pub fn admin(user: i64) -> Viewer {
        Viewer {
            user,
            admin: true,
            can_manage: true,
            scope: BankScope::All,
        }
    }
    /// May update or delete a bank owned by `owner` (`None`: no recorded owner, which only administrators may manage).
    pub fn manages(&self, owner: Option<i64>) -> bool {
        self.admin || (self.can_manage && owner == Some(self.user))
    }
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedBank {
    pub id: i64,
    /// The deleted bank was the legacy bank (the target of uploads without `X-Bank-Target`).
    pub legacy: bool,
    /// Its name (for the log)
    pub name: String,
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
