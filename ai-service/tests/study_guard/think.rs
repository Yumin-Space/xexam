//! Reasoning tags in a hint: `<think>`, `</think>`, `<thinking>` and `</thinking>` (any letter case, full-width forms
//! included) are reasoning that reached the text, so the hint is withheld, also when a tag arrives split across pieces.

use super::util::*;
use xexam_ai_service::study::guard::lists::{HOLD_MIN, SCAN_EVERY, THINK_TAGS};
use xexam_ai_service::study::guard::{HintGuard, Verdict};

type Fx = (
    xexam_ai_service::study::model::StudyQuestion,
    xexam_ai_service::study::model::Solution,
);

fn fixtures() -> Vec<(Fx, &'static str)> {
    vec![
        (single(), "Think about orbital distance first."),
        (multi(), "Rocky planets have solid surfaces."),
        (hotspot(), "Decide whether statement 2 is true or false."),
    ]
}

const PAD: &str =
    "Consider the orbital periods and how much light each body receives over a year. ";

/// A clean prefix long enough that earlier text is released before the tag arrives.
fn prefix(fx: &Fx) -> String {
    let need = 2 * HintGuard::new(&fx.0, &fx.1).hold() + SCAN_EVERY + 1;
    let mut s = String::new();
    while s.chars().count() <= need {
        s.push_str(PAD);
    }
    s
}

const TAILS: &[&str] = &[" Then compare the options calmly.", ""];

/// Feeds the pieces (text or bytes); returns (released text, leaked). After a leak every further call must leak too.
fn run(fx: &Fx, pieces: &[&[u8]], by_bytes: bool) -> (String, bool) {
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let mut out = String::new();
    let mut leaked = false;
    for p in pieces {
        let v = if by_bytes {
            g.push_bytes(p)
        } else {
            g.push(std::str::from_utf8(p).expect("char-aligned piece"))
        };
        match v {
            Verdict::Release(s) => {
                assert!(!leaked, "released text after a leak");
                out.push_str(&s);
            }
            Verdict::Leak => leaked = true,
        }
    }
    match g.finish() {
        Verdict::Release(s) => {
            assert!(!leaked, "finish released text after a leak");
            out.push_str(&s);
        }
        Verdict::Leak => leaked = true,
    }
    (out, leaked)
}

#[test]
fn tags_in_a_whole_text_leak_for_every_fixture() {
    let tags = [
        "<think>",
        "</think>",
        "<thinking>",
        "</thinking>",
        "<THINK>",
        "</Thinking>",
        "＜think＞",
        "＜／ｔｈｉｎｋ＞",
        "<think>the first option is right</think>",
    ];
    for (fx, clean) in fixtures() {
        assert!(!whole(&fx, clean), "clean: {clean:?}");
        for tag in tags {
            let mid = clean.len() / 2;
            let mid = (mid..).find(|i| clean.is_char_boundary(*i)).unwrap_or(0);
            let texts = [
                format!("{}{tag}{}", &clean[..mid], &clean[mid..]),
                format!("{tag}{clean}"),
                format!("{clean}{tag}"),
            ];
            for t in texts {
                assert!(whole(&fx, &t), "must leak: {t:?}");
            }
        }
        for ok in [
            "Think about it",
            "<think about it>",
            "< think>",
            "<thinker>",
            "<think attr>",
            "3 < 4 > 2",
        ] {
            let t = format!("{clean} {ok}");
            assert!(!whole(&fx, &t), "must not leak: {t:?}");
        }
    }
}

#[test]
fn a_streamed_tag_leaks_for_every_split() {
    for tag in ["<think>x</think>", "</thinking>", "＜think＞x＜／think＞"] {
        for (fx, _) in fixtures() {
            let pre = prefix(&fx);
            for tail in TAILS {
                let text = format!("{pre}{tag}{tail}");
                assert!(whole(&fx, &text), "whole: {tag:?}");
                // two pieces, by char (push) and by byte (push_bytes, also inside UTF-8 sequences)
                for i in (0..=text.len()).filter(|i| text.is_char_boundary(*i)) {
                    let (a, b) = text.as_bytes().split_at(i);
                    let (out, leaked) = run(&fx, &[a, b], false);
                    assert!(leaked, "char split {i} of {tag:?}");
                    assert!(
                        !out.contains('<'),
                        "released a tag start: split {i} of {tag:?}"
                    );
                }
                for i in 0..=text.len() {
                    let (a, b) = text.as_bytes().split_at(i);
                    let (out, leaked) = run(&fx, &[a, b], true);
                    assert!(leaked, "byte split {i} of {tag:?}");
                    assert!(
                        !out.contains('<'),
                        "released a tag start: byte split {i} of {tag:?}"
                    );
                }
                // one character / one byte at a time
                let chars: Vec<String> = text.chars().map(String::from).collect();
                let pieces: Vec<&[u8]> = chars.iter().map(|c| c.as_bytes()).collect();
                let (out, leaked) = run(&fx, &pieces, false);
                assert!(leaked && !out.contains('<'), "one char at a time: {tag:?}");
                let pieces: Vec<&[u8]> = text.as_bytes().chunks(1).collect();
                let (out, leaked) = run(&fx, &pieces, true);
                assert!(leaked && !out.contains('<'), "one byte at a time: {tag:?}");
            }
        }
    }
}

#[test]
fn a_lone_closing_tag_after_a_long_prefix_leaks_in_small_pieces() {
    for (fx, _) in fixtures() {
        let pre = prefix(&fx);
        let text = format!("{pre}</think>{pre}");
        for n in 1..=16usize {
            let chars: Vec<char> = text.chars().collect();
            let owned: Vec<String> = chars.chunks(n).map(|c| c.iter().collect()).collect();
            let pieces: Vec<&[u8]> = owned.iter().map(|s| s.as_bytes()).collect();
            let (out, leaked) = run(&fx, &pieces, false);
            assert!(leaked, "pieces of {n}");
            assert!(
                pre.starts_with(&out),
                "released text is a prefix of the clean part: pieces of {n}"
            );
        }
    }
}

#[test]
fn the_tag_list_and_the_hold_back_fit_together() {
    assert_eq!(
        THINK_TAGS,
        &["<think>", "</think>", "<thinking>", "</thinking>"]
    );
    let longest = THINK_TAGS.iter().map(|t| t.chars().count()).max().unwrap();
    assert!(
        HOLD_MIN >= longest,
        "the guard holds back at least one whole tag"
    );
}
