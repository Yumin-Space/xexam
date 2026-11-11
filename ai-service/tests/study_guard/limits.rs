//! Cost bound of the windowed scans and the HINT_MAX_CHARS truncation.

use super::util::*;
use xexam_ai_service::limits::HINT_MAX_CHARS;
use xexam_ai_service::study::guard::{HintGuard, Verdict};

/// The spec bound `(hold / 32 + 3) x length`, with exact (not floored) division.
fn bound_for(hold: usize, len: usize) -> u64 {
    ((hold as f64 / 32.0 + 3.0) * len as f64) as u64
}

fn filler(len: usize) -> String {
    let words = [
        "orbit", "period", "light", "distance", "year", "study", "ratio", "model",
    ];
    let mut s = String::new();
    let mut i = 0;
    while s.chars().count() < len {
        s.push_str(words[i % words.len()]);
        s.push(' ');
        i += 1;
    }
    s.chars().take(len).collect()
}

fn run(
    fx: &(
        xexam_ai_service::study::model::StudyQuestion,
        xexam_ai_service::study::model::Solution,
    ),
    text: &str,
    n: usize,
) -> (HintGuard, String, bool) {
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    for c in chars.chunks(n) {
        let piece: String = c.iter().collect();
        match g.push(&piece) {
            Verdict::Release(s) => out.push_str(&s),
            Verdict::Leak => return (g, out, true),
        }
        if g.is_truncated() {
            break;
        }
    }
    match g.finish() {
        Verdict::Release(s) => out.push_str(&s),
        Verdict::Leak => return (g, out, true),
    }
    (g, out, false)
}

#[test]
fn scan_cost_is_bounded_for_every_piece_size() {
    for fx in [single(), hotspot(), multi()] {
        let text = filler(HINT_MAX_CHARS);
        for n in [1usize, 2, 3, 7, 31, 32, 33, 50, 200, 1000, 6000] {
            let (g, out, leaked) = run(&fx, &text, n);
            assert!(!leaked, "chunk {n}");
            assert_eq!(out, text);
            let bound = bound_for(g.hold(), text.chars().count());
            assert!(
                g.scanned_chars() <= bound,
                "chunk {n}: scanned {} > bound {bound} (hold {})",
                g.scanned_chars(),
                g.hold()
            );
            // Every character is scanned at least once.
            assert!(g.scanned_chars() >= text.chars().count() as u64 - g.hold() as u64);
        }
    }
}

#[test]
fn scan_cost_is_bounded_with_a_large_hold() {
    let long: String = (0..40).map(|i| format!("word{i} ")).collect();
    let q = question(Kind::Single, "Which?", &["One", "Two"]);
    let fx = (q, sol(&[long.trim()]));
    let text = filler(HINT_MAX_CHARS);
    for n in [1usize, 5, 32, 100] {
        let (g, _, leaked) = run(&fx, &text, n);
        assert!(!leaked);
        assert!(g.hold() > 200);
        let bound = bound_for(g.hold(), HINT_MAX_CHARS);
        assert!(
            g.scanned_chars() <= bound,
            "chunk {n}: {} > {bound}",
            g.scanned_chars()
        );
    }
}

#[test]
fn scan_counter_grows_linearly_not_quadratically() {
    let fx = single();
    let (small, _, _) = run(&fx, &filler(1500), 1);
    let (big, _, _) = run(&fx, &filler(6000), 1);
    let ratio = big.scanned_chars() as f64 / small.scanned_chars() as f64;
    assert!((3.5..4.5).contains(&ratio), "ratio {ratio}");
}

#[test]
fn hold_is_clamped() {
    let q = question(Kind::Single, "Which?", &["One", "Two"]);
    assert_eq!(HintGuard::new(&q, &sol(&["One"])).hold(), 64);
    let k = "x".repeat(300);
    assert_eq!(HintGuard::new(&q, &sol(&[&k])).hold(), 348);
    let k = "x".repeat(2000);
    assert_eq!(HintGuard::new(&q, &sol(&[&k])).hold(), 600);
}

#[test]
fn a_hint_of_exactly_the_limit_is_not_truncated() {
    let fx = single();
    let text = filler(HINT_MAX_CHARS);
    let (g, out, leaked) = run(&fx, &text, 100);
    assert!(!leaked);
    assert!(!g.is_truncated());
    assert_eq!(out, text);
}

#[test]
fn a_longer_hint_is_cut_at_the_limit_after_finish() {
    let fx = single();
    let text = filler(HINT_MAX_CHARS + 700);
    for n in [1usize, 7, 100, 5999, 6000, 6001, 9000] {
        let (g, out, leaked) = run(&fx, &text, n);
        assert!(!leaked, "chunk {n}");
        assert!(g.is_truncated(), "chunk {n}");
        assert_eq!(out.chars().count(), HINT_MAX_CHARS, "chunk {n}");
        assert!(text.starts_with(&out), "chunk {n}");
        assert_eq!(g.released_chars(), HINT_MAX_CHARS);
    }
}

#[test]
fn text_after_the_limit_is_ignored() {
    let fx = single();
    let mut text = filler(HINT_MAX_CHARS);
    text.push_str(" The answer is B.");
    let (g, out, leaked) = run(&fx, &text, 50);
    assert!(!leaked, "the claim lies after the cut");
    assert!(g.is_truncated());
    assert_eq!(out.chars().count(), HINT_MAX_CHARS);
    // A claim before the cut is still found.
    let mut early = filler(HINT_MAX_CHARS - 20);
    early.push_str("The answer is B. ");
    early.push_str(&filler(300));
    let (_, _, leaked) = run(&fx, &early, 50);
    assert!(leaked);
}

#[test]
fn a_claim_cut_in_half_by_the_limit_is_no_claim() {
    let fx = single();
    let mut cut_before = filler(HINT_MAX_CHARS - 14);
    cut_before.push_str("The answer is B");
    // 14 characters of "The answer is " fit; the letter B is cut off.
    let (g, out, leaked) = run(&fx, &cut_before, 9);
    assert!(g.is_truncated());
    assert!(!leaked);
    assert!(out.ends_with("The answer is "));
    // One more character and the letter is inside.
    let mut cut_after = filler(HINT_MAX_CHARS - 15);
    cut_after.push_str("The answer is B");
    let (_, _, leaked) = run(&fx, &cut_after, 9);
    assert!(leaked);
}

#[test]
fn truncation_counts_characters_not_bytes() {
    let fx = single();
    let text = "思考".repeat(HINT_MAX_CHARS);
    let (g, out, leaked) = run(&fx, &text, 777);
    assert!(!leaked);
    assert!(g.is_truncated());
    assert_eq!(out.chars().count(), HINT_MAX_CHARS);
}

#[test]
fn pushes_after_truncation_release_nothing() {
    let fx = single();
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let _ = g.push(&filler(HINT_MAX_CHARS + 10));
    assert!(g.is_truncated());
    assert_eq!(g.push("The answer is B"), Verdict::Release(String::new()));
    assert!(matches!(g.finish(), Verdict::Release(_)));
}
