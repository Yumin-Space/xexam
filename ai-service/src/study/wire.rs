//! The backend's study data routes (`/internal/ai/question`, `/internal/ai/record`, `/internal/ai/record-info`):
//! strict parsing of their answers and conversion to the prompt model.
//!
//! An answer is first read as JSON. `{"ok": false, "reason": r}` (exactly these two keys, `r` one of the route's
//! reasons) is a refusal; `{"ok": true, …}` is deserialised with `deny_unknown_fields` into the types below and its
//! values are validated (kinds, languages, answer entries, key and time shapes). Anything else is
//! [`WireError::Unreadable`]: the caller fails only its own request (the link state is never touched).
use crate::prompt::Lang;
use crate::study::model::{
    HotspotItem, KeyAnswer, OptionText, QKind, Solution, StudyQuestion, UserEntry,
};
use crate::study::summary::{RecordKind, SummaryItem};
use crate::util::is_lower_hex;
use serde::Deserialize;
use serde_json::Value;

/// Why the backend refused a data call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The session is gone or a password change is pending
    Session,
    /// The user does not have the AI+ feature
    NotAllowed,
    /// Missing, foreign or out-of-scope (one answer for all three)
    NotFound,
}

/// A data answer that is not a good answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    Refused(Refusal),
    /// Not JSON, an unknown or missing key, a value outside its set, or an `ok`/`reason` combination of no route
    Unreadable,
}

/// The identity of a saved record (what decides its chat).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordIdent {
    pub id: i64,
    /// `YYYY-MM-DD HH:MM`, the backend's local time
    pub at: String,
    pub kind: RecordKind,
    /// The session key (32 lower-case hex) of a record saved by v0.9 or later
    pub key: Option<String>,
    pub started_at: Option<i64>,
}

/// The saved item of a record for the asked question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemInfo {
    pub seq: u32,
    /// `None` = not answered
    pub answer: Option<Vec<UserEntry>>,
    pub correct: bool,
}

/// A good `/internal/ai/question` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionReply {
    pub ui_lang: Lang,
    pub question: StudyQuestion,
    pub solution: Solution,
    pub item: Option<ItemInfo>,
    pub record: Option<RecordIdent>,
}

/// A good `/internal/ai/record` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordReply {
    pub ui_lang: Lang,
    pub record: RecordIdent,
    pub mode: String,
    pub total: i64,
    pub correct: i64,
    pub answered: Option<i64>,
    pub detail: bool,
    pub wrong: i64,
    pub usable: i64,
    pub items: Vec<SummaryItem>,
    pub budget_cut: bool,
}

/// A good `/internal/ai/record-info` answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoReply {
    pub ui_lang: Lang,
    pub record: RecordIdent,
}

// —— raw shapes ——————————————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawOption {
    key: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawChoiceItem {
    text: String,
    choices: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawQuestion {
    id: String,
    kind: String,
    lang: String,
    stem: String,
    options: Vec<RawOption>,
    items: Vec<RawChoiceItem>,
    active: bool,
    cut: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawSolution {
    answer: Vec<Value>,
    explanation: String,
    mnemonic: String,
    note: String,
    explain_lang: String,
    key_texts: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawItem {
    seq: i64,
    answer: Option<Vec<Value>>,
    correct: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawIdent {
    id: i64,
    at: String,
    kind: String,
    key: Option<String>,
    started_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawQuestionAnswer {
    ok: bool,
    ui_lang: String,
    question: RawQuestion,
    solution: RawSolution,
    item: Option<RawItem>,
    record: Option<RawIdent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawRecordHead {
    id: i64,
    at: String,
    kind: String,
    mode: String,
    total: i64,
    correct: i64,
    answered: Option<i64>,
    detail: bool,
    key: Option<String>,
    started_at: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawRecordItem {
    seq: i64,
    answer: Vec<Value>,
    question: RawQuestion,
    solution: RawSolution,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawRecordAnswer {
    ok: bool,
    ui_lang: String,
    record: RawRecordHead,
    wrong: i64,
    usable: i64,
    items: Vec<RawRecordItem>,
    budget_cut: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawInfoAnswer {
    ok: bool,
    ui_lang: String,
    record: RawIdent,
}

// —— validation and conversion ———————————————————————————————————————————————————————————————————————————————————————

type Res<T> = Result<T, WireError>;

fn bad<T>() -> Res<T> {
    Err(WireError::Unreadable)
}

/// `en` / `zh` / `ja` as the backend writes them (not the `zh-CN` alias).
fn lang_of(s: &str) -> Res<Lang> {
    match s {
        "en" | "zh" | "ja" => Lang::parse(s).map_or_else(bad, Ok),
        _ => bad(),
    }
}

fn kind_of(s: &str) -> Res<QKind> {
    match s {
        "single" => Ok(QKind::Single),
        "multi" => Ok(QKind::Multi),
        "hotspot" => Ok(QKind::Hotspot),
        _ => bad(),
    }
}

fn record_kind_of(s: &str) -> Res<RecordKind> {
    match s {
        "practice" => Ok(RecordKind::Practice),
        "exam" => Ok(RecordKind::Exam),
        _ => bad(),
    }
}

/// `YYYY-MM-DD HH:MM`.
fn at_ok(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 16
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b' ',
            13 => *c == b':',
            _ => c.is_ascii_digit(),
        })
}

fn index_of(v: &Value) -> Option<usize> {
    v.as_u64().and_then(|n| usize::try_from(n).ok())
}

fn entries(raw: &[Value]) -> Res<Vec<UserEntry>> {
    raw.iter()
        .map(|v| match v {
            Value::String(s) => Ok(UserEntry::Text(s.clone())),
            Value::Null => Ok(UserEntry::Null),
            n @ Value::Number(_) => index_of(n).map(UserEntry::Index).map_or_else(bad, Ok),
            _ => bad(),
        })
        .collect()
}

fn question(raw: RawQuestion) -> Res<StudyQuestion> {
    Ok(StudyQuestion {
        id: raw.id,
        kind: kind_of(&raw.kind)?,
        lang: lang_of(&raw.lang)?,
        stem: raw.stem,
        options: raw
            .options
            .into_iter()
            .map(|o| OptionText {
                key: o.key,
                text: o.text,
            })
            .collect(),
        items: raw
            .items
            .into_iter()
            .map(|i| HotspotItem {
                text: i.text,
                choices: i.choices,
            })
            .collect(),
        active: raw.active,
        cut: raw.cut,
    })
}

fn solution(raw: RawSolution) -> Res<Solution> {
    let answer = if raw.answer.iter().all(Value::is_string) {
        KeyAnswer::Letters(
            raw.answer
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
        )
    } else if raw.answer.iter().all(|v| index_of(v).is_some()) {
        KeyAnswer::Indexes(raw.answer.iter().filter_map(index_of).collect())
    } else {
        return bad();
    };
    let explain_lang = if raw.explain_lang.is_empty() {
        None
    } else {
        Some(lang_of(&raw.explain_lang)?)
    };
    Ok(Solution {
        answer,
        explanation: raw.explanation,
        mnemonic: raw.mnemonic,
        note: raw.note,
        explain_lang,
        key_texts: raw.key_texts,
    })
}

fn key_ok(key: &Option<String>) -> bool {
    key.as_deref().is_none_or(|k| is_lower_hex(k, 32))
}

fn ident(raw: RawIdent) -> Res<RecordIdent> {
    if raw.id <= 0 || !at_ok(&raw.at) || !key_ok(&raw.key) || raw.started_at.is_some_and(|s| s <= 0)
    {
        return bad();
    }
    Ok(RecordIdent {
        id: raw.id,
        at: raw.at,
        kind: record_kind_of(&raw.kind)?,
        key: raw.key,
        started_at: raw.started_at,
    })
}

fn seq_of(n: i64) -> Res<u32> {
    u32::try_from(n).map_or_else(|_| bad(), Ok)
}

/// Splits a refusal from a good answer: `reasons` are the refusals the route may give.
fn envelope(body: &[u8], reasons: &[(&str, Refusal)]) -> Res<Value> {
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return bad();
    };
    let Some(obj) = v.as_object() else {
        return bad();
    };
    match obj.get("ok") {
        Some(Value::Bool(true)) => Ok(v),
        Some(Value::Bool(false)) => {
            let reason = obj.get("reason").and_then(Value::as_str);
            let found = reasons.iter().find(|(r, _)| Some(*r) == reason);
            match (found, obj.len()) {
                (Some((_, refusal)), 2) => Err(WireError::Refused(*refusal)),
                _ => bad(),
            }
        }
        _ => bad(),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(v: Value) -> Res<T> {
    serde_json::from_value(v).map_err(|_| WireError::Unreadable)
}

const QUESTION_REASONS: &[(&str, Refusal)] = &[
    ("session", Refusal::Session),
    ("notAllowed", Refusal::NotAllowed),
    ("notFound", Refusal::NotFound),
];
const INFO_REASONS: &[(&str, Refusal)] = &[
    ("session", Refusal::Session),
    ("notFound", Refusal::NotFound),
];

/// Parses a `/internal/ai/question` answer. `with_record` says whether the request carried a `record`: `item` and
/// `record` are non-null exactly then.
pub fn parse_question(body: &[u8], with_record: bool) -> Res<QuestionReply> {
    let raw: RawQuestionAnswer = parse(envelope(body, QUESTION_REASONS)?)?;
    if !raw.ok || raw.item.is_some() != with_record || raw.record.is_some() != with_record {
        return bad();
    }
    let item = match raw.item {
        Some(i) => Some(ItemInfo {
            seq: seq_of(i.seq)?,
            answer: i.answer.as_deref().map(entries).transpose()?,
            correct: i.correct,
        }),
        None => None,
    };
    Ok(QuestionReply {
        ui_lang: lang_of(&raw.ui_lang)?,
        question: question(raw.question)?,
        solution: solution(raw.solution)?,
        item,
        record: raw.record.map(ident).transpose()?,
    })
}

/// Parses a `/internal/ai/record` answer. `max` is the number of items asked for.
pub fn parse_record(body: &[u8], max: usize) -> Res<RecordReply> {
    let raw: RawRecordAnswer = parse(envelope(body, QUESTION_REASONS)?)?;
    let h = raw.record;
    if !raw.ok
        || raw.items.len() > max
        || raw.wrong < 0
        || raw.usable < 0
        || raw.usable > raw.wrong
        || h.total < 0
        || h.correct < 0
        || h.answered.is_some_and(|a| a < 0)
        || h.mode.is_empty()
    {
        return bad();
    }
    let record = ident(RawIdent {
        id: h.id,
        at: h.at,
        kind: h.kind,
        key: h.key,
        started_at: h.started_at,
    })?;
    let mut items = Vec::with_capacity(raw.items.len());
    for it in raw.items {
        items.push(SummaryItem {
            seq: seq_of(it.seq)?,
            answer: entries(&it.answer)?,
            question: question(it.question)?,
            solution: solution(it.solution)?,
        });
    }
    Ok(RecordReply {
        ui_lang: lang_of(&raw.ui_lang)?,
        record,
        mode: h.mode,
        total: h.total,
        correct: h.correct,
        answered: h.answered,
        detail: h.detail,
        wrong: raw.wrong,
        usable: raw.usable,
        items,
        budget_cut: raw.budget_cut,
    })
}

/// Parses a `/internal/ai/record-info` answer.
pub fn parse_info(body: &[u8]) -> Res<InfoReply> {
    let raw: RawInfoAnswer = parse(envelope(body, INFO_REASONS)?)?;
    if !raw.ok {
        return bad();
    }
    Ok(InfoReply {
        ui_lang: lang_of(&raw.ui_lang)?,
        record: ident(raw.record)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn q(id: &str) -> Value {
        json!({"id": id, "kind": "single", "lang": "en", "stem": "Pick one",
               "options": [{"key": "A", "text": "x"}, {"key": "B", "text": "y"}],
               "items": [], "active": true, "cut": false})
    }

    fn sol() -> Value {
        json!({"answer": ["B"], "explanation": "e", "mnemonic": "", "note": "", "explainLang": "", "keyTexts": ["y"]})
    }

    fn bytes(v: &Value) -> Vec<u8> {
        serde_json::to_vec(v).expect("json")
    }

    fn record() -> Value {
        json!({"id": 7, "at": "2026-01-02 03:04", "kind": "exam", "key": "0123456789abcdef0123456789abcdef", "startedAt": 1700000000})
    }

    fn plain() -> Value {
        json!({"ok": true, "uiLang": "zh", "question": q("q1"), "solution": sol(), "item": null, "record": null})
    }

    #[test]
    fn question_answer_with_and_without_a_record() {
        let r = parse_question(&bytes(&plain()), false).expect("parsed");
        assert_eq!(r.ui_lang, Lang::Zh);
        assert_eq!(r.question.kind, QKind::Single);
        assert_eq!(r.solution.answer, KeyAnswer::Letters(vec!["B".into()]));
        assert!(r.item.is_none() && r.record.is_none());
        let with = json!({"ok": true, "uiLang": "en", "question": q("q1"), "solution": sol(),
                          "item": {"seq": 3, "answer": ["A", null], "correct": false}, "record": record()});
        // a record was not asked for, but one came back
        assert_eq!(
            parse_question(&bytes(&with), false),
            Err(WireError::Unreadable)
        );
        let r = parse_question(&bytes(&with), true).expect("parsed");
        let item = r.item.expect("item");
        assert_eq!(item.seq, 3);
        assert_eq!(
            item.answer,
            Some(vec![UserEntry::Text("A".into()), UserEntry::Null])
        );
        let rec = r.record.expect("record");
        assert_eq!(rec.kind, RecordKind::Exam);
        assert_eq!(rec.started_at, Some(1_700_000_000));
        // a record was asked for, none came back
        assert_eq!(
            parse_question(&bytes(&plain()), true),
            Err(WireError::Unreadable)
        );
        // an unanswered item
        let un = json!({"ok": true, "uiLang": "ja", "question": q("q1"), "solution": sol(),
                        "item": {"seq": 0, "answer": null, "correct": false}, "record": record()});
        assert_eq!(
            parse_question(&bytes(&un), true)
                .expect("ok")
                .item
                .map(|i| i.answer),
            Some(None)
        );
    }

    #[test]
    fn refusals_and_unreadable_answers() {
        for (reason, want) in [
            ("session", Refusal::Session),
            ("notAllowed", Refusal::NotAllowed),
            ("notFound", Refusal::NotFound),
        ] {
            let b = bytes(&json!({"ok": false, "reason": reason}));
            assert_eq!(parse_question(&b, false), Err(WireError::Refused(want)));
            assert_eq!(parse_record(&b, 70).err(), Some(WireError::Refused(want)));
        }
        // record-info has no notAllowed
        let na = bytes(&json!({"ok": false, "reason": "notAllowed"}));
        assert_eq!(parse_info(&na), Err(WireError::Unreadable));
        let nf = bytes(&json!({"ok": false, "reason": "notFound"}));
        assert_eq!(parse_info(&nf), Err(WireError::Refused(Refusal::NotFound)));
        for junk in [
            json!({"ok": false}),
            json!({"ok": false, "reason": "other"}),
            json!({"ok": false, "reason": "notFound", "extra": 1}),
            json!({"reason": "notFound"}),
            json!([1]),
            json!({"ok": 1}),
        ] {
            assert_eq!(
                parse_question(&bytes(&junk), false),
                Err(WireError::Unreadable),
                "{junk}"
            );
        }
        assert_eq!(
            parse_question(b"not json", false),
            Err(WireError::Unreadable)
        );
        // an extra field anywhere
        let mut extra = plain();
        extra["question"]["surprise"] = json!(1);
        assert_eq!(
            parse_question(&bytes(&extra), false),
            Err(WireError::Unreadable)
        );
        // values outside their sets
        for (field, v) in [("kind", json!("essay")), ("lang", json!("fr"))] {
            let mut b = plain();
            b["question"][field] = v;
            assert_eq!(
                parse_question(&bytes(&b), false),
                Err(WireError::Unreadable),
                "{field}"
            );
        }
        let mut b = plain();
        b["solution"]["answer"] = json!(["A", 1]);
        assert_eq!(
            parse_question(&bytes(&b), false),
            Err(WireError::Unreadable)
        );
        b["solution"]["answer"] = json!([0, 1]);
        let ok = parse_question(&bytes(&b), false).expect("indexes");
        assert_eq!(ok.solution.answer, KeyAnswer::Indexes(vec![0, 1]));
        b["solution"]["answer"] = json!([-1]);
        assert_eq!(
            parse_question(&bytes(&b), false),
            Err(WireError::Unreadable)
        );
        b["solution"]["answer"] = json!(["A"]);
        b["solution"]["explainLang"] = json!("de");
        assert_eq!(
            parse_question(&bytes(&b), false),
            Err(WireError::Unreadable)
        );
        b["solution"]["explainLang"] = json!("ja");
        assert_eq!(
            parse_question(&bytes(&b), false)
                .expect("ja")
                .solution
                .explain_lang,
            Some(Lang::Ja)
        );
    }

    #[test]
    fn record_and_info_answers() {
        let item = json!({"seq": 2, "answer": ["A"], "question": q("q9"), "solution": sol()});
        let head = json!({"id": 7, "at": "2026-01-02 03:04", "kind": "practice", "mode": "practice", "total": 10,
                          "correct": 4, "answered": 9, "detail": true, "key": null, "startedAt": null});
        let a = json!({"ok": true, "uiLang": "en", "record": head, "wrong": 5, "usable": 4, "items": [item], "budgetCut": false});
        let r = parse_record(&bytes(&a), 70).expect("record");
        assert_eq!((r.wrong, r.usable, r.items.len()), (5, 4, 1));
        assert_eq!(r.items[0].seq, 2);
        assert_eq!(r.record.key, None);
        // more items than asked for, usable above wrong, a bad time, a bad key
        assert_eq!(
            parse_record(&bytes(&a), 0).err(),
            Some(WireError::Unreadable)
        );
        let mut b = a.clone();
        b["usable"] = json!(6);
        assert_eq!(
            parse_record(&bytes(&b), 70).err(),
            Some(WireError::Unreadable)
        );
        let mut b = a.clone();
        b["record"]["at"] = json!("2026-01-02T03:04");
        assert_eq!(
            parse_record(&bytes(&b), 70).err(),
            Some(WireError::Unreadable)
        );
        let mut b = a.clone();
        b["record"]["key"] = json!("NOTHEX");
        assert_eq!(
            parse_record(&bytes(&b), 70).err(),
            Some(WireError::Unreadable)
        );
        let info = json!({"ok": true, "uiLang": "ja", "record": record()});
        let i = parse_info(&bytes(&info)).expect("info");
        assert_eq!(i.record.id, 7);
        assert_eq!(i.ui_lang, Lang::Ja);
    }
}
