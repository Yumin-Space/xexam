//! Shared text rendering: question blocks, answers, escaping and clipping.
use crate::study::model::{KeyAnswer, QKind, Solution, StudyQuestion, UserEntry};

/// The placeholder for a missing entry of a hotspot answer.
pub const DASH: &str = "—";
/// The text of an answer with nothing in it.
pub const NONE: &str = "(none)";

/// Embedded text can never close a data tag: every `</` becomes `< /` (spec 5.7).
pub fn esc(s: &str) -> String {
    s.replace("</", "< /")
}

/// At most `max` characters; a longer text is cut and ends with `…` (the whole result is still `max` characters).
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// The question text without tags: stem, a blank line, then `A. text` lines (hotspot: `1. statement — choices: a / b`).
pub fn question_body(q: &StudyQuestion) -> String {
    let mut lines: Vec<String> = Vec::new();
    match q.kind {
        QKind::Hotspot => {
            for (i, it) in q.items.iter().enumerate() {
                lines.push(format!(
                    "{}. {} — choices: {}",
                    i + 1,
                    esc(&it.text),
                    esc(&it.choices.join(" / "))
                ));
            }
        }
        _ => {
            for o in &q.options {
                lines.push(format!("{}. {}", esc(&o.key), esc(&o.text)));
            }
        }
    }
    if lines.is_empty() {
        esc(&q.stem)
    } else {
        format!("{}\n\n{}", esc(&q.stem), lines.join("\n"))
    }
}

fn type_label(kind: QKind) -> &'static str {
    match kind {
        QKind::Single => "single choice",
        QKind::Multi => "multiple choice (one or more answers)",
        QKind::Hotspot => "statements, each with its own choices",
    }
}

/// The `<question>` block of a user turn.
pub fn question_block(q: &StudyQuestion, no: u32) -> String {
    format!(
        "<question number=\"{no}\" type=\"{}\" language=\"{}\">\n{}\n</question>",
        type_label(q.kind),
        q.lang.question_name(),
        question_body(q)
    )
}

fn choice_text(q: &StudyQuestion, item: usize, idx: usize) -> Option<&str> {
    q.items
        .get(item)
        .and_then(|it| it.choices.get(idx))
        .map(String::as_str)
}

/// The answer key, as `B`, `A, C` or `1: Yes; 2: No`.
pub fn format_key(q: &StudyQuestion, sol: &Solution) -> String {
    match &sol.answer {
        KeyAnswer::Letters(l) => l.iter().map(|s| esc(s)).collect::<Vec<_>>().join(", "),
        KeyAnswer::Indexes(ix) => ix
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                format!(
                    "{}: {}",
                    i + 1,
                    choice_text(q, i, c).map(esc).unwrap_or_else(|| DASH.into())
                )
            })
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// The longest choice text rendered inside a user-answer line.
pub const USER_CHOICE_MAX: usize = 80;

/// One option letter `A`..`H` (the only text a stored single / multiple choice answer may contain).
fn is_option_letter(s: &str) -> bool {
    matches!(s.as_bytes(), [b'A'..=b'H'])
}

/// Single-line, clipped, escaped text: safe for the system prompt, where stored text is least welcome.
fn one_line(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    esc(&clip(&flat, USER_CHOICE_MAX))
}

/// A stored user answer, as `A, C`, `1: Yes; 2: —; 3: No` or `(none)`.
///
/// Single / multiple choice: only entries that are exactly one letter `A`..`H` are kept (anything else is dropped, since
/// this text also goes into the system prompt), joined by `, `. Hotspot: one `n: text` per item of the
/// question (`—` for a null entry, a missing entry or an entry of the wrong type); an empty list or a list without
/// any usable entry is `(none)`.
pub fn format_user(q: &StudyQuestion, entries: &[UserEntry]) -> String {
    match q.kind {
        QKind::Hotspot => {
            if q.items.is_empty() || !entries.iter().any(|e| matches!(e, UserEntry::Index(_))) {
                return NONE.to_string();
            }
            (0..q.items.len())
                .map(|i| {
                    let text = match entries.get(i) {
                        Some(UserEntry::Index(c)) => choice_text(q, i, *c).map(one_line),
                        _ => None,
                    };
                    format!("{}: {}", i + 1, text.unwrap_or_else(|| DASH.into()))
                })
                .collect::<Vec<_>>()
                .join("; ")
        }
        _ => {
            let letters: Vec<String> = entries
                .iter()
                .filter_map(|e| match e {
                    UserEntry::Text(s) if is_option_letter(s) => Some(s.clone()),
                    _ => None,
                })
                .collect();
            if letters.is_empty() {
                NONE.to_string()
            } else {
                letters.join(", ")
            }
        }
    }
}
