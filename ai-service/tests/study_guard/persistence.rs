//! The binding persistence rule: a guarded run only ever stores and sends
//! released text, whatever way it ends.

use super::util::*;
use xexam_ai_service::study::guard::{HintGuard, Verdict};

#[derive(Clone, Copy, Debug)]
enum Outcome {
    Done,
    Failed,
    Cancelled,
}

/// Result of the simulated `chat::run` of spec 5.8.
struct Run {
    /// What was sent as `delta` events (and counted as shown).
    sent: String,
    /// What the final transaction stores as `content`.
    stored: String,
    /// `Some("ai.hintLeak")` when the guard withheld the hint.
    error_code: Option<&'static str>,
}

/// Feeds `pieces`; the upstream then ends with `outcome`; `finish()` runs on
/// every outcome before the final transaction.
fn simulate(
    fx: &(
        xexam_ai_service::study::model::StudyQuestion,
        xexam_ai_service::study::model::Solution,
    ),
    pieces: &[&str],
    outcome: Outcome,
) -> Run {
    let _ = outcome; // the guard treats every outcome alike
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let mut sent = String::new();
    let withheld = |sent: String| Run {
        sent,
        stored: String::new(),
        error_code: Some("ai.hintLeak"),
    };
    for p in pieces {
        match g.push(p) {
            Verdict::Release(s) => sent.push_str(&s),
            Verdict::Leak => return withheld(sent),
        }
    }
    match g.finish() {
        Verdict::Release(s) => sent.push_str(&s),
        Verdict::Leak => return withheld(sent),
    }
    Run {
        stored: sent.clone(),
        sent,
        error_code: None,
    }
}

#[test]
fn clean_text_is_stored_in_full_for_every_outcome() {
    let fx = single();
    let pieces = [
        "Think about orbital ",
        "distance, then compare ",
        "the periods of each body.",
    ];
    for outcome in [Outcome::Done, Outcome::Failed, Outcome::Cancelled] {
        let r = simulate(&fx, &pieces, outcome);
        assert_eq!(r.error_code, None);
        assert_eq!(r.stored, pieces.concat());
        assert_eq!(r.sent, r.stored);
    }
}

#[test]
fn a_leak_stores_nothing_for_every_outcome() {
    let fx = single();
    let pad = "Consider the orbital periods and how much light each body receives. ".repeat(4);
    let text = format!("{pad}The answer is B. {pad}");
    for outcome in [Outcome::Done, Outcome::Failed, Outcome::Cancelled] {
        for n in [1usize, 4, 9, 40] {
            let chars: Vec<char> = text.chars().collect();
            let pieces: Vec<String> = chars.chunks(n).map(|c| c.iter().collect()).collect();
            let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
            let r = simulate(&fx, &refs, outcome);
            assert_eq!(r.error_code, Some("ai.hintLeak"), "{outcome:?} {n}");
            assert_eq!(r.stored, "", "{outcome:?} {n}");
            // Nothing that was sent contains the claim, or any part of it
            // that points at an option.
            assert!(!whole(&fx, &r.sent), "{outcome:?} {n}: {:?}", r.sent);
            assert!(!r.sent.contains("is B"), "{outcome:?} {n}: {:?}", r.sent);
            assert!(text.starts_with(&r.sent));
        }
    }
}

#[test]
fn a_short_leaking_hint_releases_nothing_before_finish() {
    let fx = single();
    let mut g = HintGuard::new(&fx.0, &fx.1);
    for p in ["The ", "answer ", "is ", "B"] {
        assert_eq!(g.push(p), Verdict::Release(String::new()), "{p:?}");
    }
    assert_eq!(g.released_chars(), 0);
    assert_eq!(g.finish(), Verdict::Leak);
}

#[test]
fn a_claim_cut_by_an_error_is_released_as_the_text_it_is() {
    let fx = single();
    let r = simulate(&fx, &["Think first. The answer is"], Outcome::Failed);
    assert_eq!(r.error_code, None);
    assert_eq!(r.stored, "Think first. The answer is");
}

#[test]
fn a_leak_is_final() {
    let fx = single();
    let mut g = HintGuard::new(&fx.0, &fx.1);
    assert_eq!(
        g.push("Pick D now, and then some more text to scan."),
        Verdict::Leak
    );
    assert_eq!(g.push("harmless"), Verdict::Leak);
    assert_eq!(g.push_bytes(b"harmless"), Verdict::Leak);
    assert_eq!(g.finish(), Verdict::Leak);
    assert_eq!(g.finish(), Verdict::Leak);
}

#[test]
fn finish_is_idempotent_and_push_after_finish_releases_nothing() {
    let fx = single();
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let _ = g.push("Think about distance.");
    assert!(matches!(g.finish(), Verdict::Release(_)));
    assert_eq!(g.finish(), Verdict::Release(String::new()));
    assert_eq!(g.push("The answer is B"), Verdict::Release(String::new()));
}

#[test]
fn released_text_is_always_a_prefix_and_trails_by_the_hold() {
    let fx = single();
    let text = filler_ascii(1200);
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let hold = g.hold();
    let mut out = String::new();
    let mut fed = 0;
    for chunk in text.as_bytes().chunks(5) {
        let piece = std::str::from_utf8(chunk).expect("ascii");
        fed += piece.len();
        match g.push(piece) {
            Verdict::Release(s) => out.push_str(&s),
            Verdict::Leak => panic!("unexpected leak"),
        }
        assert!(text.starts_with(&out));
        // Unchecked text never leaves: at least `hold` characters stay inside.
        assert!(
            fed - out.len() >= hold.min(fed),
            "fed {fed}, released {}",
            out.len()
        );
        assert_eq!(g.released_chars(), out.chars().count());
    }
    match g.finish() {
        Verdict::Release(s) => out.push_str(&s),
        Verdict::Leak => panic!("unexpected leak"),
    }
    assert_eq!(out, text);
}

fn filler_ascii(len: usize) -> String {
    let words = ["orbit", "period", "light", "ratio", "year", "model"];
    let mut s = String::new();
    let mut i = 0;
    while s.len() < len {
        s.push_str(words[i % words.len()]);
        s.push(' ');
        i += 1;
    }
    s.truncate(len);
    s
}

#[test]
fn released_pieces_never_carry_a_whole_claim_for_random_leaks() {
    let fx = single();
    let claims = [
        "The answer is B.",
        "Go with the second option.",
        "Mars is wrong.",
        "答案是B。",
        "Compare with C.",
    ];
    let mut rng = Lcg(99);
    for case in 0..400 {
        let pre = filler_ascii(rng.next(300));
        let post = filler_ascii(rng.next(300));
        let claim = claims[case % claims.len()];
        let text = format!("{pre} {claim} {post}");
        let mut g = HintGuard::new(&fx.0, &fx.1);
        let mut sent = String::new();
        let mut leaked = false;
        let mut rest = text.as_str();
        while !rest.is_empty() {
            let n = 1 + rng.next(30);
            let cut = rest.char_indices().nth(n).map_or(rest.len(), |(i, _)| i);
            match g.push(&rest[..cut]) {
                Verdict::Release(s) => sent.push_str(&s),
                Verdict::Leak => {
                    leaked = true;
                    break;
                }
            }
            rest = &rest[cut..];
        }
        if !leaked {
            leaked = matches!(g.finish(), Verdict::Leak);
        }
        assert!(leaked, "case {case}: {text:?}");
        assert!(!whole(&fx, &sent), "case {case}: {sent:?}");
    }
}
