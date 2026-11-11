//! Shared fixtures and helpers (all questions are self-made samples).
#![allow(dead_code)]

pub use xexam_ai_service::prompt::Lang;
use xexam_ai_service::study::guard::{leaks, HintGuard, Verdict};
pub use xexam_ai_service::study::model::QKind as Kind;
use xexam_ai_service::study::model::{HotspotItem, KeyAnswer, OptionText, Solution, StudyQuestion};

pub fn opts(texts: &[&str]) -> Vec<OptionText> {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| OptionText {
            key: ((b'A' + i as u8) as char).to_string(),
            text: (*t).to_string(),
        })
        .collect()
}

/// A full question value with neutral defaults for everything the guard does not read.
pub fn question_full(
    kind: Kind,
    stem: &str,
    options: Vec<OptionText>,
    items: Vec<HotspotItem>,
) -> StudyQuestion {
    StudyQuestion {
        id: "q-guard".to_string(),
        kind,
        lang: Lang::En,
        stem: stem.to_string(),
        options,
        items,
        active: true,
        cut: false,
    }
}

pub fn question(kind: Kind, stem: &str, texts: &[&str]) -> StudyQuestion {
    question_full(kind, stem, opts(texts), Vec::new())
}

pub fn sol(keys: &[&str]) -> Solution {
    Solution {
        answer: KeyAnswer::Letters(Vec::new()),
        explanation: String::new(),
        mnemonic: String::new(),
        note: String::new(),
        explain_lang: None,
        key_texts: keys.iter().map(|s| (*s).to_string()).collect(),
    }
}

/// Four options, correct answer A (Mercury), texts in two languages.
pub fn single() -> (StudyQuestion, Solution) {
    (
        question(
            Kind::Single,
            "Which planet is closest to the sun?",
            &["Mercury", "Venus", "Earth", "Mars"],
        ),
        sol(&["Mercury", "水星", "The nearest planet to the Sun"]),
    )
}

/// Same with a stem in which the letter B stands alone ("Class B").
pub fn single_class_b() -> (StudyQuestion, Solution) {
    (
        question(
            Kind::Single,
            "Which planet is closest to the sun, as seen from a Class B observatory?",
            &["Mercury", "Venus", "Earth", "Mars"],
        ),
        sol(&["Mercury"]),
    )
}

pub fn multi() -> (StudyQuestion, Solution) {
    (
        question(
            Kind::Multi,
            "Select all rocky planets.",
            &["Mercury", "Gas giant", "Earth", "Neptune"],
        ),
        sol(&["Mercury", "Earth"]),
    )
}

/// Eight options so every English ordinal is a valid option reference.
pub fn eight() -> (StudyQuestion, Solution) {
    (
        question(
            Kind::Single,
            "Pick the odd number out.",
            &[
                "one-ish",
                "two-ish",
                "three-ish",
                "four-ish",
                "five-ish",
                "six-ish",
                "seven-ish",
                "eight-ish",
            ],
        ),
        sol(&["one-ish"]),
    )
}

/// Three hotspot statements; items 1 and 2 are yes / no, item 3 names a node.
pub fn hotspot() -> (StudyQuestion, Solution) {
    let item = |text: &str, choices: &[&str]| HotspotItem {
        text: text.to_string(),
        choices: choices.iter().map(|c| (*c).to_string()).collect(),
    };
    (
        question_full(
            Kind::Hotspot,
            "Review the statements about the cluster.",
            Vec::new(),
            vec![
                item("The server is online", &["Yes", "No"]),
                item("Backups run nightly", &["Yes", "No"]),
                item("Traffic is served by", &["web-node", "db-node"]),
            ],
        ),
        sol(&["Yes", "No", "db-node"]),
    )
}

/// Whole-text verdict for a fixture.
pub fn whole(fx: &(StudyQuestion, Solution), text: &str) -> bool {
    leaks(text, &fx.0, &fx.1.key_texts)
}

/// Streams `pieces` through a guard; (released text, leaked).
pub fn stream(fx: &(StudyQuestion, Solution), pieces: &[&str]) -> (String, bool) {
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let mut out = String::new();
    for p in pieces {
        match g.push(p) {
            Verdict::Release(s) => out.push_str(&s),
            Verdict::Leak => return (out, true),
        }
    }
    match g.finish() {
        Verdict::Release(s) => {
            out.push_str(&s);
            (out, false)
        }
        Verdict::Leak => (out, true),
    }
}

/// Streams text split into chunks of `n` characters.
pub fn stream_chunks(fx: &(StudyQuestion, Solution), text: &str, n: usize) -> (String, bool) {
    let chars: Vec<char> = text.chars().collect();
    let pieces: Vec<String> = chars.chunks(n).map(|c| c.iter().collect()).collect();
    let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
    stream(fx, &refs)
}

/// Streams raw bytes split into chunks of `n` bytes.
pub fn stream_bytes(fx: &(StudyQuestion, Solution), text: &str, n: usize) -> (String, bool) {
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let mut out = String::new();
    for chunk in text.as_bytes().chunks(n) {
        match g.push_bytes(chunk) {
            Verdict::Release(s) => out.push_str(&s),
            Verdict::Leak => return (out, true),
        }
    }
    match g.finish() {
        Verdict::Release(s) => {
            out.push_str(&s);
            (out, false)
        }
        Verdict::Leak => (out, true),
    }
}

/// Tiny deterministic generator for the property tests.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}
