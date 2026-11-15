//! The content hash of a shared hint / explanation (`study_shared.content_hash`).
//!
//! The hash covers everything the prompt is built from, so an edited question (stem, option, answer key, bank
//! explanation) simply stops matching its old row. It carries nothing of a user: the personal sentence of the result
//! page is not part of a shared prompt.
use crate::study::explain::EXPLAIN_PROMPT_V;
use crate::study::hint::HINT_PROMPT_V;
use crate::study::model::{KeyAnswer, QKind, Solution, StudyQuestion};
use crate::util::hex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// The wire spelling of a question kind.
fn kind_text(k: QKind) -> &'static str {
    match k {
        QKind::Single => "single",
        QKind::Multi => "multi",
        QKind::Hotspot => "hotspot",
    }
}

/// The prompt version of a shared kind (`hint` | `explain`).
pub fn prompt_version(kind: &str) -> u32 {
    if kind == "hint" {
        HINT_PROMPT_V
    } else {
        EXPLAIN_PROMPT_V
    }
}

fn key_value(a: &KeyAnswer) -> Value {
    match a {
        KeyAnswer::Letters(l) => json!(l),
        KeyAnswer::Indexes(i) => json!(i),
    }
}

/// 32 lower-case hex digits: the first 16 bytes of SHA-256 over the canonical JSON
/// `[PROMPT_V, kind, qlang, q.kind, stem, [[key, text]...], [[item text, [choices]]...], cut]` and, for `explain`
/// only (`sol` is `Some`), `+ [answer key, explanation, mnemonic, note]`. `qlang` is the language the question texts
/// are really in.
pub fn content_hash(kind: &str, q: &StudyQuestion, sol: Option<&Solution>) -> String {
    let options: Vec<Value> = q.options.iter().map(|o| json!([o.key, o.text])).collect();
    let items: Vec<Value> = q.items.iter().map(|i| json!([i.text, i.choices])).collect();
    let mut parts = vec![
        json!(prompt_version(kind)),
        json!(kind),
        json!(super::lang_code(q.lang)),
        json!(kind_text(q.kind)),
        json!(q.stem),
        Value::Array(options),
        Value::Array(items),
        json!(q.cut),
    ];
    if let Some(s) = sol {
        parts.push(key_value(&s.answer));
        parts.push(json!(s.explanation));
        parts.push(json!(s.mnemonic));
        parts.push(json!(s.note));
    }
    let digest = Sha256::digest(Value::Array(parts).to_string().as_bytes());
    hex(&digest[..16])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::Lang;
    use crate::study::explain::EXPLAIN_SYSTEM;
    use crate::study::hint::HINT_SYSTEM;
    use crate::study::model::{HotspotItem, OptionText};

    fn q() -> StudyQuestion {
        StudyQuestion {
            id: "q1".into(),
            kind: QKind::Single,
            lang: Lang::En,
            stem: "Which colour is the sky on a clear day?".into(),
            options: vec![
                OptionText {
                    key: "A".into(),
                    text: "Blue".into(),
                },
                OptionText {
                    key: "B".into(),
                    text: "Green".into(),
                },
            ],
            items: vec![],
            active: true,
            cut: false,
        }
    }

    fn sol() -> Solution {
        Solution {
            answer: KeyAnswer::Letters(vec!["A".into()]),
            explanation: "Scattering.".into(),
            mnemonic: String::new(),
            note: String::new(),
            explain_lang: None,
            key_texts: vec![],
        }
    }

    fn digest(s: &str) -> String {
        hex(&Sha256::digest(s.as_bytes())[..8])
    }

    #[test]
    fn hash_is_32_hex_and_stable() {
        let h = content_hash("hint", &q(), None);
        assert_eq!(h.len(), 32);
        assert!(h
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_eq!(h, content_hash("hint", &q(), None));
    }

    #[test]
    fn hash_changes_with_the_question_content() {
        let base = content_hash("hint", &q(), None);
        let mut a = q();
        a.stem.push('!');
        assert_ne!(base, content_hash("hint", &a, None));
        let mut b = q();
        b.options[1].text = "Grey".into();
        assert_ne!(base, content_hash("hint", &b, None));
        let mut c = q();
        c.lang = Lang::Zh;
        assert_ne!(base, content_hash("hint", &c, None));
        let mut d = q();
        d.cut = true;
        assert_ne!(base, content_hash("hint", &d, None));
        let mut e = q();
        e.kind = QKind::Multi;
        assert_ne!(base, content_hash("hint", &e, None));
        let mut f = q();
        f.items.push(HotspotItem {
            text: "It rains".into(),
            choices: vec!["Yes".into(), "No".into()],
        });
        assert_ne!(base, content_hash("hint", &f, None));
        // A hint and an explanation never share a hash
        assert_ne!(base, content_hash("explain", &q(), None));
    }

    #[test]
    fn explain_hash_covers_the_solution() {
        let base = content_hash("explain", &q(), Some(&sol()));
        let mut s = sol();
        s.answer = KeyAnswer::Letters(vec!["B".into()]);
        assert_ne!(base, content_hash("explain", &q(), Some(&s)));
        let mut s = sol();
        s.explanation.push('.');
        assert_ne!(base, content_hash("explain", &q(), Some(&s)));
        let mut s = sol();
        s.mnemonic = "aid".into();
        assert_ne!(base, content_hash("explain", &q(), Some(&s)));
        let mut s = sol();
        s.note = "n".into();
        assert_ne!(base, content_hash("explain", &q(), Some(&s)));
        let mut s = sol();
        s.answer = KeyAnswer::Indexes(vec![0]);
        assert_ne!(base, content_hash("explain", &q(), Some(&s)));
    }

    #[test]
    fn prompt_version_is_part_of_the_hash() {
        assert_eq!(prompt_version("hint"), HINT_PROMPT_V);
        assert_eq!(prompt_version("explain"), EXPLAIN_PROMPT_V);
    }

    #[test]
    fn prompt_texts_are_tied_to_their_version() {
        // Editing a prompt text must bump the matching `*_PROMPT_V` (old shared rows then stop matching); after the
        // bump, put the new digest and version here.
        assert_eq!(
            (HINT_PROMPT_V, digest(HINT_SYSTEM).as_str()),
            (1, HINT_DIGEST)
        );
        assert_eq!(
            (EXPLAIN_PROMPT_V, digest(EXPLAIN_SYSTEM).as_str()),
            (1, EXPLAIN_DIGEST)
        );
    }

    const HINT_DIGEST: &str = "27f1bbac375491ca";
    const EXPLAIN_DIGEST: &str = "97a95734b511e3e3";
}
