//! Validation of one parsed question against its slot (GP-4). The first failing rule is the reason.
//!
//! C1 (the slot belongs to the batch and is not filled twice in one answer) is the caller's job (`run.rs`); `Copy`
//! and `Duplicate` come from `dedup.rs`.

use std::collections::HashSet;

use crate::gen::dedup::norm;
use crate::gen::parse::RawQuestion;
use crate::gen::plan::Slot;
use crate::gen::text::{
    chars, has_letter_ref, has_option_label, has_positional, looks_chinese, looks_english,
    looks_japanese,
};
use crate::gen::Tri;

/// Why a question was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reject {
    UnknownSlot,
    DuplicateSlot,
    OptionCount,
    CorrectCount,
    Empty,
    TooShort,
    TooLong,
    Language,
    Untranslated,
    DupOption,
    PositionalOption,
    LetterRef,
    Copy,
    Duplicate,
    // The four below are used only by the import's conversion checks (IA-4)
    NotVerbatim,
    OptionOrder,
    AnswerMismatch,
    TrueFalse,
}

impl Reject {
    /// The wire word of a rejection reason (logs and tests).
    pub fn code(self) -> &'static str {
        match self {
            Reject::UnknownSlot => "unknownSlot",
            Reject::DuplicateSlot => "duplicateSlot",
            Reject::OptionCount => "optionCount",
            Reject::CorrectCount => "correctCount",
            Reject::Empty => "empty",
            Reject::TooShort => "tooShort",
            Reject::TooLong => "tooLong",
            Reject::Language => "language",
            Reject::Untranslated => "untranslated",
            Reject::DupOption => "dupOption",
            Reject::PositionalOption => "positionalOption",
            Reject::LetterRef => "letterRef",
            Reject::Copy => "copy",
            Reject::Duplicate => "duplicate",
            Reject::NotVerbatim => "notVerbatim",
            Reject::OptionOrder => "optionOrder",
            Reject::AnswerMismatch => "answerMismatch",
            Reject::TrueFalse => "trueFalse",
        }
    }
}

pub const STEM_MIN: usize = 8;
pub const STEM_MIN_EN: usize = 20;
pub const STEM_MAX: usize = 800;
pub const OPTION_MIN: usize = 1;
pub const OPTION_MAX: usize = 300;
pub const EXPL_MIN: usize = 8;
pub const EXPL_MIN_EN: usize = 20;
pub const EXPL_MAX: usize = 1200;

fn length_rule(t: &Tri, min: usize, min_en: usize, max: usize) -> Result<(), Reject> {
    for (k, s) in t.all().iter().enumerate() {
        let n = chars(s.trim());
        let lo = if k == 0 { min_en } else { min };
        if n < lo {
            return Err(Reject::TooShort);
        }
        if n > max {
            return Err(Reject::TooLong);
        }
    }
    Ok(())
}

pub(crate) fn language_rule(t: &Tri) -> Result<(), Reject> {
    if looks_english(&t.en) && looks_chinese(&t.zh) && looks_japanese(&t.ja) {
        Ok(())
    } else {
        Err(Reject::Language)
    }
}

/// C2 .. C8 for one question.
pub fn check(q: &RawQuestion, slot: &Slot) -> Result<(), Reject> {
    // C2, C3
    if q.options.len() != slot.opts as usize {
        return Err(Reject::OptionCount);
    }
    if q.options.iter().filter(|o| o.correct).count() != slot.correct.len() {
        return Err(Reject::CorrectCount);
    }
    // C4: non-empty first, then the lengths.
    let empty = |t: &Tri| t.all().iter().any(|s| s.trim().is_empty());
    if empty(&q.stem) || empty(&q.explanation) || q.options.iter().any(|o| empty(&o.text)) {
        return Err(Reject::Empty);
    }
    length_rule(&q.stem, STEM_MIN, STEM_MIN_EN, STEM_MAX)?;
    for o in &q.options {
        length_rule(&o.text, OPTION_MIN, OPTION_MIN, OPTION_MAX)?;
    }
    length_rule(&q.explanation, EXPL_MIN, EXPL_MIN_EN, EXPL_MAX)?;
    // C5: a stem copied into another language is `Untranslated` (checked first: such a copy also fails the language
    // ratios, and the copy is the more useful reason); then the language ratios of stem and explanation.
    if norm(&q.stem.zh) == norm(&q.stem.ja) || norm(&q.stem.en) == norm(&q.stem.zh) {
        return Err(Reject::Untranslated);
    }
    language_rule(&q.stem)?;
    language_rule(&q.explanation)?;
    // C6: options distinct within each language.
    for lang in 0..3 {
        let mut seen = HashSet::new();
        for o in &q.options {
            if !seen.insert(norm(o.text.all()[lang])) {
                return Err(Reject::DupOption);
            }
        }
    }
    // C7
    if q.options
        .iter()
        .any(|o| o.text.all().iter().any(|s| has_positional(s)))
    {
        return Err(Reject::PositionalOption);
    }
    // C8: letter references in explanations and options, labelled options.
    if q.explanation.all().iter().any(|s| has_letter_ref(s))
        || q.options.iter().any(|o| {
            o.text
                .all()
                .iter()
                .any(|s| has_letter_ref(s) || has_option_label(s))
        })
    {
        return Err(Reject::LetterRef);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::parse::RawOption;
    use crate::gen::testkit::*;
    use crate::gen::Kind;

    fn slot(opts: u8, correct: Vec<u8>) -> Slot {
        Slot {
            no: 1,
            stratum: 0,
            kind: if correct.len() > 1 {
                Kind::Multi
            } else {
                Kind::Single
            },
            opts,
            correct,
            seed: "x".into(),
            diff: None,
        }
    }

    fn good(s: &Slot) -> RawQuestion {
        raw_for(s, 7)
    }

    #[test]
    fn a_valid_question_passes() {
        let s = slot(4, vec![2]);
        assert_eq!(check(&good(&s), &s), Ok(()));
        let m = slot(5, vec![0, 3]);
        assert_eq!(check(&good(&m), &m), Ok(()));
    }

    #[test]
    fn option_and_correct_counts() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.options.pop();
        assert_eq!(check(&q, &s), Err(Reject::OptionCount));
        let mut q = good(&s);
        q.options[1].correct = true;
        assert_eq!(check(&q, &s), Err(Reject::CorrectCount));
        let mut q = good(&s);
        q.options[0].correct = false;
        assert_eq!(check(&q, &s), Err(Reject::CorrectCount));
    }

    #[test]
    fn empty_and_length_rules() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.stem.zh = "   ".into();
        assert_eq!(check(&q, &s), Err(Reject::Empty));
        let mut q = good(&s);
        q.options[2].text.ja.clear();
        assert_eq!(check(&q, &s), Err(Reject::Empty));
        let mut q = good(&s);
        q.stem.en = "Too short".into();
        assert_eq!(check(&q, &s), Err(Reject::TooShort));
        let mut q = good(&s);
        q.stem.zh = "短".repeat(7);
        assert_eq!(check(&q, &s), Err(Reject::TooShort));
        let mut q = good(&s);
        q.stem.zh = "长".repeat(801);
        assert_eq!(check(&q, &s), Err(Reject::TooLong));
        let mut q = good(&s);
        q.options[0].text.en = "x".repeat(301);
        assert_eq!(check(&q, &s), Err(Reject::TooLong));
        let mut q = good(&s);
        q.explanation.ja = "あ".repeat(1201);
        assert_eq!(check(&q, &s), Err(Reject::TooLong));
        let mut q = good(&s);
        q.explanation.en = "short expl".into();
        assert_eq!(check(&q, &s), Err(Reject::TooShort));
    }

    #[test]
    fn boundaries_are_accepted() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.stem.zh = "中".repeat(8);
        q.stem.en = "a".repeat(20);
        q.options[0].text.en = "x".repeat(300);
        q.options[1].text.zh = "项".to_string();
        q.explanation.zh = "说".repeat(1200);
        assert_eq!(check(&q, &s), Ok(()));
    }

    #[test]
    fn language_rules() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.stem.en = han(1, 30); // Chinese text in the English field
        assert_eq!(check(&q, &s), Err(Reject::Language));
        let mut q = good(&s);
        q.stem.zh = tri(2, 10).ja; // kana in the Chinese field
        assert_eq!(check(&q, &s), Err(Reject::Language));
        let mut q = good(&s);
        q.stem.ja = tri(3, 10).zh; // no kana in the Japanese field
        assert_eq!(check(&q, &s), Err(Reject::Language));
        let mut q = good(&s);
        q.explanation.en = kana(4, 30);
        assert_eq!(check(&q, &s), Err(Reject::Language));
    }

    #[test]
    fn mixed_technical_terms_are_fine() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.stem.en = format!("{} and the SKU-12 plus 蒸气 label", en_text(40, 6));
        q.stem.zh = format!("{} 以及 SKU-12 和 steam", han(40, 8));
        q.stem.ja = format!("{} と SKU-12", kana(40, 8));
        assert_eq!(check(&q, &s), Ok(()));
    }

    #[test]
    fn untranslated_stems() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.stem.zh = q.stem.en.clone();
        assert_eq!(check(&q, &s), Err(Reject::Untranslated));
        let mut q = good(&s);
        q.stem.zh = format!("{}。", q.stem.ja);
        assert_eq!(check(&q, &s), Err(Reject::Untranslated));
        assert_eq!(norm("Ab, c!"), norm("a B C"));
    }

    #[test]
    fn duplicate_options_per_language() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.options[2].text.zh = format!("{}！", q.options[0].text.zh);
        assert_eq!(check(&q, &s), Err(Reject::DupOption));
        let mut q = good(&s);
        let first = q.options[0].text.en.to_uppercase();
        q.options[3].text.en = first;
        assert_eq!(check(&q, &s), Err(Reject::DupOption));
    }

    #[test]
    fn positional_options_are_refused() {
        let s = slot(4, vec![1]);
        for text in ["All of the above", "以上都对", "上記のすべて"] {
            let mut q = good(&s);
            q.options[3].text.en = text.into();
            assert_eq!(check(&q, &s), Err(Reject::PositionalOption), "{text}");
        }
    }

    #[test]
    fn letter_references_are_refused() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.explanation.en = format!("{} so Option B is wrong", en_text(50, 5));
        assert_eq!(check(&q, &s), Err(Reject::LetterRef));
        let mut q = good(&s);
        q.explanation.zh = format!("{}选项 C", han(50, 8));
        assert_eq!(check(&q, &s), Err(Reject::LetterRef));
        let mut q = good(&s);
        q.explanation.ja = format!("{}Ａが正しい", kana(50, 8));
        assert_eq!(check(&q, &s), Err(Reject::LetterRef));
        let mut q = good(&s);
        q.options[1].text.en = "A. Steam oven".into();
        assert_eq!(check(&q, &s), Err(Reject::LetterRef));
        let mut q = good(&s);
        q.options[1].text.zh = "（C）蒸气".into();
        assert_eq!(check(&q, &s), Err(Reject::LetterRef));
    }

    #[test]
    fn letter_like_words_pass() {
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.explanation.en = format!(
            "{} Plan A is cheaper and the IPv6 address works; the answer a user gives is stored",
            en_text(60, 4)
        );
        q.options[0].text.en = "A/B testing".into();
        q.options[2].text.en = "B2B order".into();
        assert_eq!(check(&q, &s), Ok(()));
    }

    #[test]
    fn order_of_rules() {
        // Several faults at once: the first rule in the order wins.
        let s = slot(4, vec![1]);
        let mut q = good(&s);
        q.options.push(RawOption {
            text: tri(99, 2),
            correct: false,
        });
        q.stem.en = String::new();
        assert_eq!(check(&q, &s), Err(Reject::OptionCount));
        let mut q = good(&s);
        q.stem.en = String::new();
        q.options[3].text.en = "All of the above".into();
        assert_eq!(check(&q, &s), Err(Reject::Empty));
    }
}
