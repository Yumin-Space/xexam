//! One positive test per list entry: the tests iterate the constants, so a new
//! entry is covered the moment it is added (and a negative control proves the
//! entry, not something else, causes the leak).

use super::util::*;
use xexam_ai_service::study::guard::lists::*;
use xexam_ai_service::study::model::{HotspotItem, Solution, StudyQuestion};

fn hotspot10() -> (StudyQuestion, Solution) {
    let items = (0..10)
        .map(|i| HotspotItem {
            text: format!("Statement text number {i}"),
            choices: vec!["Yes".to_string(), "No".to_string()],
        })
        .collect();
    (
        question_full(Kind::Hotspot, "Review the statements.", Vec::new(), items),
        sol(&["Yes", "No"]),
    )
}

#[test]
fn list_sizes_are_pinned() {
    assert_eq!(LEADS.len(), 39);
    assert_eq!(TRAILS.len(), 55);
    assert_eq!(TRUTH_WORDS.len(), 28);
    assert_eq!(ORDINALS_EN.len(), 8);
    assert_eq!(ORDINALS_EN_10.len(), 10);
    assert_eq!(CONNECTOR_WORDS.len(), 50);
    assert_eq!(HINT_MAX_CHARS, 6_000);
    assert_eq!(LEAD_GAP, 40);
    assert_eq!(TRAIL_PUNCT, 6);
    assert_eq!(CONNECTOR_MAX, 8);
    assert_eq!(LOOSE_GAP, 20);
    assert_eq!(QUOTE_MIN_CHARS, 12);
    assert_eq!(QUOTE_MIN_CJK, 5);
    assert_eq!(X_OPTION_MAX, 120);
    assert_eq!(SCAN_EVERY, 32);
}

#[test]
fn every_lead_flags_a_following_option_text() {
    // The stem names the key text, so Rule 2b does not apply and only the
    // lead words can flag it.
    let fx = (
        question(
            Kind::Single,
            "Which planet is closest to the sun, Mercury or another?",
            &["Mercury", "Venus", "Earth", "Mars"],
        ),
        sol(&["Mercury"]),
    );
    assert!(!whole(&fx, "x Mercury"));
    for lead in LEADS {
        let t = format!("x {lead} Mercury");
        assert!(whole(&fx, &t), "lead {lead:?}: {t:?}");
        // Also directly adjacent, as in CJK writing.
        let t = format!("x{lead}Mercury");
        if lead.chars().all(|c| !c.is_ascii_alphanumeric()) {
            assert!(whole(&fx, &t), "lead {lead:?}: {t:?}");
        }
    }
}

#[test]
fn every_trail_flags_a_preceding_option_text() {
    let fx = single();
    for trail in TRAILS {
        let t = format!("Mercury {trail}");
        assert!(whole(&fx, &t), "trail {trail:?}: {t:?}");
        let t = format!("Mercury{trail}");
        if trail.chars().all(|c| !c.is_ascii_alphanumeric()) {
            assert!(whole(&fx, &t), "trail {trail:?}: {t:?}");
        }
    }
}

#[test]
fn every_punct_char_may_sit_between_target_and_trail() {
    let fx = single();
    for p in PUNCT {
        let t = format!("Mercury{p}is wrong");
        assert!(whole(&fx, &t), "punct {p:?}");
    }
}

#[test]
fn every_english_ordinal_is_an_option_reference() {
    let fx = eight();
    assert!(!whole(&fx, "go with the first thing"));
    for o in ORDINALS_EN.iter().chain(&["last"]) {
        let t = format!("go with the {o} option");
        assert!(whole(&fx, &t), "{t:?}");
        let t = format!("The {o} choice is wrong");
        assert!(whole(&fx, &t), "{t:?}");
    }
}

#[test]
fn every_ordinal_noun_completes_a_reference() {
    let fx = eight();
    for n in ORDINAL_NOUNS {
        let t = format!("go with the second {n}");
        assert!(whole(&fx, &t), "{t:?}");
        let t = format!("go with the second real {n}");
        assert!(whole(&fx, &t), "{t:?}");
    }
}

#[test]
fn every_truth_word_is_a_claim_word() {
    let fx = hotspot();
    assert!(!whole(&fx, "Statement 1 fine"));
    for w in TRUTH_WORDS {
        let t = format!("Statement 1 {w}");
        assert!(whole(&fx, &t), "{t:?}");
        let t = format!("{w} statement 2");
        assert!(whole(&fx, &t), "{t:?}");
    }
}

#[test]
fn every_single_cjk_word_needs_context() {
    let fx = hotspot();
    for c in SINGLE_TRUTH {
        assert!(whole(&fx, &format!("第1项选{c}关于")), "{c}");
        assert!(!whole(&fx, &format!("第1项{c}关于")), "{c}");
        assert!(!whole(&fx, &format!("第1项关于{c}关")), "{c}");
    }
}

#[test]
fn every_preceder_licenses_a_single_cjk_word() {
    let fx = hotspot();
    for p in SINGLE_PRECEDERS {
        let t = format!("第1项{p}是关于");
        assert!(whole(&fx, &t), "{t:?}");
    }
}

#[test]
fn every_clause_end_licenses_a_single_cjk_word() {
    let fx = hotspot();
    for e in CLAUSE_ENDS {
        let t = format!("第1项对{e}");
        assert!(whole(&fx, &t), "{t:?}");
    }
    // Full-width forms are normalised to the same characters.
    for t in [
        "第1项对。",
        "第1项对，",
        "第1项对；",
        "第1项对）",
        "第1项对！",
        "第1项对？",
    ] {
        assert!(whole(&fx, t), "{t:?}");
    }
}

#[test]
fn every_connector_word_and_char_links_reference_and_claim() {
    let fx = hotspot();
    assert!(!whole(&fx, "Statement 1 depends true"));
    for w in CONNECTOR_WORDS {
        let t = format!("Statement 1 {w} true");
        assert!(whole(&fx, &t), "{t:?}");
    }
    for c in CONNECTOR_CHARS {
        let t = format!("Statement 1{c}true");
        assert!(whole(&fx, &t), "{t:?}");
    }
    for p in PUNCT {
        let t = format!("Statement 1{p}yes");
        assert!(whole(&fx, &t), "{t:?}");
    }
}

#[test]
fn every_english_statement_ordinal_and_noun() {
    let fx = hotspot10();
    for o in ORDINALS_EN_10 {
        for n in ITEM_NOUNS {
            let t = format!("the {o} {n} is true");
            assert!(whole(&fx, &t), "{t:?}");
        }
    }
    let small = hotspot();
    assert!(!whole(&small, "the fourth statement is true"));
    assert!(whole(&small, "the third statement is true"));
}

#[test]
fn every_letter_noun_makes_a_bare_key_letter_a_reference() {
    let fx = single_class_b();
    for n in LETTER_NOUNS_EN {
        assert!(whole(&fx, &format!("B {n}")), "{n}");
        // `A` before an English noun is an article.
        assert!(!whole(&fx, &format!("A {n} of words")), "{n}");
    }
    for n in LETTER_NOUNS_CJK {
        assert!(whole(&fx, &format!("B{n}")), "{n}");
        assert!(whole(&fx, &format!("A {n}")), "{n}");
    }
}
