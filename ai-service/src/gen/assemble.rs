//! Placing the answers at the planned positions and building the final create body (GP-6, API-I3).

use serde_json::{json, Map, Value};

use crate::gen::parse::RawQuestion;
use crate::gen::plan::Slot;
use crate::gen::{Kind, Tri, SHEET};

/// A checked question in its final shape: options in position order, answer letters `A`, `B`, ...
#[derive(Debug, Clone, PartialEq)]
pub struct FinalQuestion {
    pub kind: Kind,
    pub stem: Tri,
    pub options: Vec<Tri>,
    pub answer: Vec<String>,
    pub explanation: Tri,
}

fn trim_tri(t: &Tri) -> Tri {
    Tri {
        en: t.en.trim().to_string(),
        zh: t.zh.trim().to_string(),
        ja: t.ja.trim().to_string(),
    }
}

/// The model's correct options (in the model's order) go to the slot's `correct` positions (ascending), the wrong ones
/// fill the other positions in the model's order; the letters follow the positions. Whatever order the model used, the
/// answer positions are the planned, balanced ones. Expects a question that passed `check` (the counts match the
/// slot); on a mismatch it still returns a well-formed question instead of panicking.
pub fn place(q: &RawQuestion, slot: &Slot) -> FinalQuestion {
    let n = q.options.len();
    let mut right = q
        .options
        .iter()
        .filter(|o| o.correct)
        .map(|o| trim_tri(&o.text));
    let mut wrong = q
        .options
        .iter()
        .filter(|o| !o.correct)
        .map(|o| trim_tri(&o.text));
    let mut options: Vec<Tri> = Vec::with_capacity(n);
    let mut answer: Vec<String> = Vec::new();
    for pos in 0..n {
        let want_right = slot.correct.contains(&(pos as u8));
        let next = if want_right {
            right.next().or_else(|| wrong.next())
        } else {
            wrong.next().or_else(|| right.next())
        };
        if let Some(t) = next {
            if want_right {
                answer.push(letter(pos));
            }
            options.push(t);
        }
    }
    FinalQuestion {
        kind: slot.kind,
        stem: trim_tri(&q.stem),
        options,
        answer,
        explanation: trim_tri(&q.explanation),
    }
}

fn letter(pos: usize) -> String {
    ((b'A' + pos as u8) as char).to_string()
}

fn tri_value(t: &Tri) -> Value {
    json!({"en": t.en, "zh": t.zh, "ja": t.ja})
}

/// Exactly the body of the backend's create route (API-I3); the key order is the documented one.
#[allow(clippy::too_many_arguments)]
pub fn payload(
    job: &str,
    user: i64,
    stamp: &str,
    name: &str,
    source: &str,
    from: &[i64],
    goal: usize,
    at: &str,
    questions: &[FinalQuestion],
) -> Value {
    let list: Vec<Value> = questions
        .iter()
        .map(|q| {
            let mut m = Map::new();
            m.insert("kind".into(), json!(q.kind.as_str()));
            m.insert("stem".into(), tri_value(&q.stem));
            m.insert(
                "options".into(),
                Value::Array(q.options.iter().map(tri_value).collect()),
            );
            m.insert("answer".into(), json!(q.answer));
            m.insert("explanation".into(), tri_value(&q.explanation));
            Value::Object(m)
        })
        .collect();
    let mut m = Map::new();
    m.insert("job".into(), json!(job));
    m.insert("user".into(), json!(user));
    m.insert("stamp".into(), json!(stamp));
    m.insert("name".into(), json!(name));
    m.insert("source".into(), json!(source));
    m.insert("from".into(), json!(from));
    m.insert("goal".into(), json!(goal));
    m.insert("at".into(), json!(at));
    m.insert("sheet".into(), json!(SHEET));
    m.insert("questions".into(), Value::Array(list));
    Value::Object(m)
}

/// The create body of an AI file import (API-I3a): the key order is the documented one. `skipped` is `Some` exactly in
/// questions mode (the 1-based numbers, in the file's order); `goal` = the questions found (questions mode) or the
/// planned count (material mode); `questions.len() + skipped.len() == goal` in questions mode.
#[allow(clippy::too_many_arguments)]
pub fn payload_file(
    job: &str,
    user: i64,
    stamp: &str,
    name: &str,
    file_name: &str,
    file_type: &str,
    mode: &str,
    skipped: Option<&[usize]>,
    goal: usize,
    at: &str,
    questions: &[FinalQuestion],
) -> Value {
    let Value::Object(base) = payload(job, user, stamp, name, "file", &[], goal, at, questions)
    else {
        return Value::Null;
    };
    let mut m = Map::new();
    for (k, v) in base {
        m.insert(k.clone(), v);
        if k == "from" {
            m.insert("file".into(), json!({"name": file_name, "type": file_type}));
            m.insert("mode".into(), json!(mode));
            if let Some(s) = skipped {
                m.insert("skipped".into(), json!(s));
            }
        }
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::parse::RawOption;
    use crate::gen::testkit::*;

    fn slot(kind: Kind, opts: u8, correct: Vec<u8>) -> Slot {
        Slot {
            no: 1,
            stratum: 0,
            kind,
            opts,
            correct,
            seed: "x".into(),
            diff: None,
        }
    }

    fn raw(flags: &[bool]) -> RawQuestion {
        RawQuestion {
            slot: 1,
            stem: Tri::new(" stem ", "题", "問"),
            options: flags
                .iter()
                .enumerate()
                .map(|(k, &c)| RawOption {
                    text: Tri::new(&format!(" o{k} "), &format!("z{k}"), &format!("j{k}")),
                    correct: c,
                })
                .collect(),
            explanation: Tri::new("e ", "解", "説"),
        }
    }

    #[test]
    fn single_answer_moves_to_the_planned_position() {
        let q = raw(&[true, false, false, false]);
        let f = place(&q, &slot(Kind::Single, 4, vec![2]));
        assert_eq!(f.answer, vec!["C"]);
        assert_eq!(f.options[2].en, "o0");
        assert_eq!(f.options[0].en, "o1");
        assert_eq!(f.options[1].en, "o2");
        assert_eq!(f.options[3].en, "o3");
        assert_eq!(f.stem.en, "stem");
        assert_eq!(f.explanation.en, "e");
        assert_eq!(f.kind, Kind::Single);
    }

    #[test]
    fn multi_answers_keep_the_models_order_and_get_ascending_letters() {
        let q = raw(&[false, true, false, true, false]);
        let f = place(&q, &slot(Kind::Multi, 5, vec![0, 4]));
        assert_eq!(f.answer, vec!["A", "E"]);
        assert_eq!(f.options[0].en, "o1");
        assert_eq!(f.options[4].en, "o3");
        assert_eq!(f.options[1].en, "o0");
        assert_eq!(f.options[2].en, "o2");
        assert_eq!(f.options[3].en, "o4");
    }

    #[test]
    fn placement_is_independent_of_the_models_order() {
        let s = slot(Kind::Single, 4, vec![3]);
        for flags in [
            [true, false, false, false],
            [false, true, false, false],
            [false, false, false, true],
        ] {
            let f = place(&raw(&flags), &s);
            assert_eq!(f.answer, vec!["D"]);
        }
    }

    #[test]
    fn a_mismatching_question_does_not_panic() {
        let f = place(&raw(&[true, true, true]), &slot(Kind::Single, 3, vec![1]));
        assert_eq!(f.options.len(), 3);
        let g = place(&raw(&[false, false]), &slot(Kind::Single, 2, vec![0]));
        assert_eq!(g.options.len(), 2);
    }

    #[test]
    fn payload_has_the_documented_shape() {
        let s = slot(Kind::Single, 4, vec![1]);
        let q = place(&raw_for(&s, 3), &s);
        let v = payload(
            "0123456789abcdef0123456789abcdef",
            12,
            "fedcba9876543210fedcba9876543210",
            "Kitchen basics_alexandr_20261112093000",
            "bank",
            &[3],
            1,
            "2026-11-12 09:30:00",
            std::slice::from_ref(&q),
        );
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "job",
                "user",
                "stamp",
                "name",
                "source",
                "from",
                "goal",
                "at",
                "sheet",
                "questions"
            ]
        );
        assert_eq!(v["sheet"], "AI");
        assert_eq!(v["from"], json!([3]));
        assert_eq!(v["user"], 12);
        let qk: Vec<&str> = v["questions"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert_eq!(qk, vec!["kind", "stem", "options", "answer", "explanation"]);
        assert_eq!(v["questions"][0]["answer"], json!(["B"]));
        assert_eq!(v["questions"][0]["options"].as_array().unwrap().len(), 4);
        assert_eq!(
            v["questions"][0]["options"][0],
            json!({"en": q.options[0].en, "zh": q.options[0].zh, "ja": q.options[0].ja})
        );
    }
}
