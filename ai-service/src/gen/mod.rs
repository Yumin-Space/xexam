//! Pure generation logic for AI-generated banks (v1.1, spec GP-0 .. GP-9).
//!
//! Nothing in this module tree does I/O, reads a clock, keeps global state or draws randomness on its own: the job
//! seed, the prompt nonce and the time stamp are inputs. Siblings refer to each other as `crate::gen::...`, so this
//! folder drops into `ai-service/src/gen/` unchanged.

pub mod assemble;
pub mod check;
pub mod dedup;
pub mod parse;
pub mod plan;
pub mod prompt;
pub mod rng;
pub mod run;
pub mod runner;
pub mod text;
pub mod wire;

use serde::{Deserialize, Serialize};

/// Question type of a generated question (hotspot is never generated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Single,
    Multi,
}

impl Kind {
    /// The wire / prompt word.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Single => "single",
            Kind::Multi => "multi",
        }
    }
}

/// Difficulty label derived from aggregate statistics of the source (API-I1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Diff {
    Easy,
    Mid,
    Hard,
}

impl Diff {
    /// The word used in the prompt (`mid` is written "medium").
    pub fn prompt_word(self) -> &'static str {
        match self {
            Diff::Easy => "easy",
            Diff::Mid => "medium",
            Diff::Hard => "hard",
        }
    }
}

/// A text in the three languages of a bank.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tri {
    pub en: String,
    pub zh: String,
    pub ja: String,
}

impl Tri {
    pub fn new(en: &str, zh: &str, ja: &str) -> Tri {
        Tri {
            en: en.to_string(),
            zh: zh.to_string(),
            ja: ja.to_string(),
        }
    }

    /// The three texts in the order en, zh, ja.
    pub fn all(&self) -> [&str; 3] {
        [&self.en, &self.zh, &self.ja]
    }
}

/// One source question as API-I1 lists it (stems cut to `SOURCE_STEM_CHARS`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceItem {
    pub id: String,
    pub stratum: usize,
    pub kind: Kind,
    pub opts: u8,
    pub right: u8,
    #[serde(default)]
    pub diff: Option<Diff>,
    pub stem: Tri,
}

/// One sheet of the source (the stratum of the fair sampling).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stratum {
    pub bank: i64,
    pub sheet: String,
    /// True number of single / multiple-choice questions of the sheet.
    pub n: usize,
    /// How many of them are multiple choice.
    pub multi: usize,
}

/// Number of questions per type (admin settings, A0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Composition {
    pub single: usize,
    pub multi: usize,
}

/// A full source question as API-I2 returns it (a seed of the prompt).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedFull {
    pub id: String,
    pub sheet: String,
    pub kind: Kind,
    pub stem: Tri,
    pub options: Vec<Tri>,
    /// 0-based positions of the correct options.
    pub answer: Vec<u8>,
    pub explanation: Tri,
    #[serde(default)]
    pub diff: Option<Diff>,
}

/// Name of the one sheet of a generated bank.
pub const SHEET: &str = "AI";
pub const MAX_GOAL: usize = 100;
/// One trilingual question with explanation, as JSON.
pub const EST_CHARS_PER_QUESTION: usize = 1600;
pub const MAX_PER_CALL: usize = 5;
// Caps (characters per language) of bank text put into a prompt; the backend cuts API-I2 seeds to the same values.
pub const SEED_STEM_CHARS: usize = 600;
pub const SEED_OPTION_CHARS: usize = 150;
pub const SEED_EXPL_CHARS: usize = 400;
pub const SHEET_CHARS: usize = 80;
/// API-I1 item stems (dedup only, never in a prompt).
pub const SOURCE_STEM_CHARS: usize = 160;
/// ALREADY WRITTEN: the newest stems.
pub const WRITTEN_MAX: usize = 100;
pub const WRITTEN_CHARS: usize = 120;
/// GP-8: system + user text of one call at every cap.
pub const PROMPT_MAX_CHARS: usize = 48_000;

/// Most options of an imported question (IA-4).
pub const MAX_OPTIONS_IMPORT: usize = 8;
/// Import caps per language (IA-4, K5): stem, option, explanation.
pub const IMPORT_STEM_CHARS: usize = 2000;
pub const IMPORT_OPTION_CHARS: usize = 400;
pub const IMPORT_EXPL_CHARS: usize = 1500;

pub const NAME_MAX_HOME: usize = 60;
pub const NAME_MAX_BANK: usize = 120;

/// Why a bank name is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    Control,
}

/// Invisible format characters that are never allowed in names and are dropped from prompt / model text: bidi marks,
/// overrides and isolates, zero-width space, word joiner, BOM. U+200C / U+200D stay (emoji sequences and some scripts
/// need them).
pub fn is_hidden_format(c: char) -> bool {
    matches!(c as u32,
        0x061C | 0x200B | 0x200E | 0x200F | 0x202A..=0x202E | 0x2060..=0x2069 | 0xFEFF)
}

/// Validates a name typed by the user (Home): trimmed, 1 ..= `NAME_MAX_HOME` characters (Unicode scalar values), no
/// control or invisible format characters. Checked in the order Empty, TooLong, Control. Note: Rust's `trim` does not
/// strip U+FEFF while JavaScript's does; U+FEFF is in the format list, so the Rust side answers `Control` for it.
pub fn check_name(raw: &str) -> Result<String, NameError> {
    check_name_max(raw, NAME_MAX_HOME)
}

/// Like [`check_name`] with the bank limit `NAME_MAX_BANK` (120): the name an import confirm step pre-fills can be as long
/// as any generated bank name (`bank_mode_name`).
pub fn check_bank_name(raw: &str) -> Result<String, NameError> {
    check_name_max(raw, NAME_MAX_BANK)
}

fn check_name_max(raw: &str, max: usize) -> Result<String, NameError> {
    let name = raw.trim();
    let len = name.chars().count();
    if len == 0 {
        return Err(NameError::Empty);
    }
    if len > max {
        return Err(NameError::TooLong);
    }
    if name.chars().any(|c| c.is_control() || is_hidden_format(c)) {
        return Err(NameError::Control);
    }
    Ok(name.to_string())
}

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !is_hidden_format(*c))
        .collect()
}

/// The name of a bank generated from a source bank: `<source>_<first 8 characters of the user name>_<at>`, `at` being
/// the 14 digits of `YYYYMMDDHHMMSS`. Control / format characters are dropped from both parts, the source is trimmed
/// and cut from its end so that the whole name stays within `NAME_MAX_BANK` characters.
pub fn bank_mode_name(source: &str, username: &str, at: &str) -> String {
    let user: String = strip(username).chars().take(8).collect();
    let suffix = format!("_{user}_{at}");
    let room = NAME_MAX_BANK.saturating_sub(suffix.chars().count());
    let src = strip(source);
    let head: String = src.trim().chars().take(room).collect();
    format!("{head}{suffix}")
}

/// The name of an uploaded file made safe for a bank name and the System log (SEC-11): control and invisible format
/// characters (the bidi controls U+202A-202E and U+2066-2069 among them, U+FEFF) are dropped, `/`, `\` and `:` become
/// `_`, leading dots are trimmed, the ends are trimmed. At most 200 characters are kept.
pub fn clean_file_name(raw: &str) -> String {
    let swapped: String = strip(raw)
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let t = swapped.trim().trim_start_matches('.').trim();
    t.chars().take(200).collect::<String>().trim().to_string()
}

/// Shared helpers of the unit tests: made-up kitchen-equipment data only, never real question text.
#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use crate::gen::parse::{RawOption, RawQuestion};
    use crate::gen::plan::Slot;

    const CONS: &[u8] = b"bcdfghjklmprstvwz";
    const VOW: &[u8] = b"aeiou";

    /// A unique pseudo word (six letters, no real English word, never a keyword of the checks).
    pub fn pw(n: usize) -> String {
        let mut x = n;
        let mut s = String::new();
        for _ in 0..3 {
            s.push(CONS[x % CONS.len()] as char);
            x /= CONS.len();
            s.push(VOW[x % VOW.len()] as char);
            x /= VOW.len();
        }
        s
    }

    pub fn en_text(n: usize, words: usize) -> String {
        (0..words)
            .map(|k| pw(n * words + k))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn han(n: usize, len: usize) -> String {
        (0..len)
            .filter_map(|k| char::from_u32(0x4E00 + ((n * len + k) % 20000) as u32))
            .collect()
    }

    pub fn kana(n: usize, len: usize) -> String {
        (0..len)
            .filter_map(|k| char::from_u32(0x3042 + ((n * len + k) % 80) as u32))
            .collect()
    }

    /// A trilingual text that passes the language checks; different `n` give clearly different texts.
    pub fn tri(n: usize, words: usize) -> Tri {
        Tri {
            en: en_text(n, words),
            zh: han(n, words),
            ja: kana(n, words),
        }
    }

    /// A valid raw question for a slot: the model lists its correct options first.
    pub fn raw_for(slot: &Slot, n: usize) -> RawQuestion {
        let c = slot.correct.len();
        let options = (0..slot.opts as usize)
            .map(|k| RawOption {
                text: tri(n * 10 + k, 2),
                correct: k < c,
            })
            .collect();
        RawQuestion {
            slot: slot.no,
            stem: tri(n * 3, 10),
            options,
            explanation: tri(n * 3 + 1, 10),
        }
    }

    /// The JSON text a well-behaved model would answer for these raw questions.
    pub fn answer_json(raws: &[RawQuestion]) -> String {
        let qs: Vec<serde_json::Value> = raws
            .iter()
            .map(|q| {
                serde_json::json!({
                    "slot": q.slot,
                    "stem": {"en": q.stem.en, "zh": q.stem.zh, "ja": q.stem.ja},
                    "options": q.options.iter().map(|o| serde_json::json!(
                        {"en": o.text.en, "zh": o.text.zh, "ja": o.text.ja, "correct": o.correct})).collect::<Vec<_>>(),
                    "explanation": {"en": q.explanation.en, "zh": q.explanation.zh, "ja": q.explanation.ja},
                })
            })
            .collect();
        serde_json::json!({ "questions": qs }).to_string()
    }

    pub fn strata3() -> Vec<Stratum> {
        vec![
            Stratum {
                bank: 1,
                sheet: "Ovens".into(),
                n: 20,
                multi: 4,
            },
            Stratum {
                bank: 1,
                sheet: "Knives".into(),
                n: 10,
                multi: 0,
            },
            Stratum {
                bank: 1,
                sheet: "Storage".into(),
                n: 10,
                multi: 3,
            },
        ]
    }

    /// Made-up source items matching `strata3` (a sample of each sheet; the strata carry the true sizes).
    pub fn items3() -> Vec<SourceItem> {
        let mut v = Vec::new();
        for (s, count, multi_every) in [(0usize, 12usize, 4usize), (1, 8, 0), (2, 8, 3)] {
            for k in 0..count {
                let multi = multi_every > 0 && k % multi_every == 0;
                v.push(SourceItem {
                    id: format!("s{s}x{k}"),
                    stratum: s,
                    kind: if multi { Kind::Multi } else { Kind::Single },
                    opts: if multi { 5 } else { 4 },
                    right: if multi { 2 + (k % 2) as u8 } else { 1 },
                    diff: match k % 3 {
                        0 => Some(Diff::Easy),
                        1 => Some(Diff::Mid),
                        _ => Some(Diff::Hard),
                    },
                    stem: tri(5000 + s * 100 + k, 10),
                });
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(clean_file_name("../../x.pdf"), "_.._x.pdf");
        assert_eq!(clean_file_name("..hidden.txt"), "hidden.txt");
        assert_eq!(clean_file_name("a\\b:c/d.md"), "a_b_c_d.md");
        assert_eq!(clean_file_name("evil\u{202E}fdp.exe"), "evilfdp.exe");
        assert_eq!(clean_file_name("\u{FEFF}note\u{200B}.txt"), "note.txt");
        assert_eq!(
            clean_file_name("  spaced name .docx  "),
            "spaced name .docx"
        );
        let cjk = "漢".repeat(300);
        assert_eq!(clean_file_name(&cjk).chars().count(), 200);
        assert_eq!(clean_file_name("...."), "");
        // the bank name built from a 200-character stem still fits 120 characters
        let name = bank_mode_name(&clean_file_name(&cjk), "alexandra_k", "20261112093000");
        assert!(
            name.chars().count() <= NAME_MAX_BANK,
            "{}",
            name.chars().count()
        );
        assert!(name.ends_with("_alexandr_20261112093000"));
    }

    #[test]
    fn hidden_format_list() {
        for cp in [
            0x061Cu32, 0x200B, 0x200E, 0x200F, 0x202A, 0x202E, 0x2060, 0x2066, 0x2069, 0xFEFF,
        ] {
            assert!(is_hidden_format(char::from_u32(cp).unwrap()), "{cp:x}");
        }
        for c in [
            '\u{200C}', '\u{200D}', 'a', ' ', '中', '\u{202F}', '\u{206A}',
        ] {
            assert!(!is_hidden_format(c), "{c:?}");
        }
    }

    #[test]
    fn check_name_accepts_and_trims() {
        assert_eq!(check_name("  Kitchen set 2  ").unwrap(), "Kitchen set 2");
        assert_eq!(check_name("厨房设备题集").unwrap(), "厨房设备题集");
    }

    #[test]
    fn check_name_empty_and_length() {
        assert_eq!(check_name(""), Err(NameError::Empty));
        assert_eq!(check_name("   \t "), Err(NameError::Empty));
        let ok = "题".repeat(60);
        assert_eq!(check_name(&ok).unwrap().chars().count(), 60);
        assert_eq!(check_name(&"题".repeat(61)), Err(NameError::TooLong));
    }

    #[test]
    fn check_bank_name_allows_120() {
        assert_eq!(
            check_bank_name(&"题".repeat(120)).unwrap().chars().count(),
            120
        );
        assert_eq!(check_bank_name(&"题".repeat(121)), Err(NameError::TooLong));
        assert_eq!(check_bank_name(" "), Err(NameError::Empty));
        assert_eq!(check_bank_name("a\u{7}b"), Err(NameError::Control));
    }

    #[test]
    fn check_name_counts_code_points_not_utf16_units() {
        let emoji60 = "😀".repeat(60);
        assert_eq!(emoji60.encode_utf16().count(), 120);
        assert!(check_name(&emoji60).is_ok());
        assert_eq!(check_name(&"😀".repeat(61)), Err(NameError::TooLong));
    }

    #[test]
    fn check_name_refuses_control_and_hidden_characters() {
        assert_eq!(check_name("abc\u{202E}def"), Err(NameError::Control));
        assert_eq!(check_name("abc\u{200B}def"), Err(NameError::Control));
        assert_eq!(check_name("abc\u{0000}def"), Err(NameError::Control));
        assert_eq!(check_name("ab\ncd"), Err(NameError::Control));
        assert_eq!(check_name("ab\u{FEFF}cd"), Err(NameError::Control));
    }

    #[test]
    fn check_name_accepts_zero_width_joiner_in_emoji_sequence() {
        let family = "👩\u{200D}🍳 chef";
        assert_eq!(check_name(family).unwrap(), family);
    }

    #[test]
    fn bank_mode_name_example() {
        assert_eq!(
            bank_mode_name("Kitchen basics", "alexandra_k", "20261112093000"),
            "Kitchen basics_alexandr_20261112093000"
        );
    }

    #[test]
    fn bank_mode_name_strips_and_trims() {
        assert_eq!(
            bank_mode_name(
                "  Kit\u{202E}chen\u{200B} \n",
                "al\u{200B}ex",
                "20261112093000"
            ),
            "Kitchen_alex_20261112093000"
        );
        assert_eq!(
            bank_mode_name("Set", "bo", "20261112093000"),
            "Set_bo_20261112093000"
        );
    }

    #[test]
    fn bank_mode_name_cut_keeps_the_suffix() {
        let src = "题".repeat(200);
        let name = bank_mode_name(&src, "alexandra_k", "20261112093000");
        assert_eq!(name.chars().count(), NAME_MAX_BANK);
        assert!(name.ends_with("_alexandr_20261112093000"));
        let short = bank_mode_name(&"x".repeat(96), "alexandra_k", "20261112093000");
        assert_eq!(short.chars().count(), 96 + 24);
    }

    #[test]
    fn kind_and_diff_words() {
        assert_eq!(Kind::Multi.as_str(), "multi");
        assert_eq!(Diff::Mid.prompt_word(), "medium");
        assert_eq!(serde_json::to_string(&Diff::Mid).unwrap(), "\"mid\"");
        assert_eq!(serde_json::to_string(&Kind::Single).unwrap(), "\"single\"");
    }

    #[test]
    fn testkit_pseudo_words_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for n in 0..5000 {
            assert!(seen.insert(pw_for_test(n)));
        }
    }

    fn pw_for_test(n: usize) -> String {
        testkit::pw(n)
    }
}
