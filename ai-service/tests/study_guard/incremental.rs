//! Incremental feeding: the streamed verdict equals the whole-text verdict for
//! every way of splitting, including inside UTF-8 sequences and inside a claim.

use super::util::*;
use xexam_ai_service::study::guard::{HintGuard, Verdict};

type Fx = (
    xexam_ai_service::study::model::StudyQuestion,
    xexam_ai_service::study::model::Solution,
);

fn samples() -> Vec<(Fx, &'static str, bool)> {
    vec![
        (single(), "The answer is B.", true),
        (single(), "答案是B。", true),
        (single(), "正解はBです。", true),
        (single(), "Go with the second option, please.", true),
        (single(), "第3项是错的", true),
        (single(), "Mars can be ruled out.", true),
        (
            single(),
            "The nearest planet to the Sun is the one to find.",
            true,
        ),
        (single(), "Think about orbital distance first.", false),
        (single(), "轨道越小，周期越短。", false),
        (
            single(),
            "A key idea: shorter orbits are nearer the star.",
            false,
        ),
        (single_class_b(), "Remember what Class B means here.", false),
        (multi(), "Pick A and C.", true),
        (multi(), "Rocky planets have solid surfaces.", false),
        (hotspot(), "Statement 1 is true.", true),
        (hotspot(), "第1项选是", true),
        (hotspot(), "第1项：是。", true),
        (hotspot(), "第1项是关于服务器状态的描述。", false),
        (hotspot(), "Statement 3 involves the web-node.", true),
        (
            hotspot(),
            "Statement 1 describes whether the server is online.",
            false,
        ),
        (hotspot(), "Yes for statement 3.", true),
        (hotspot(), "Statement 2 is not true.", true),
        (hotspot(), "In statement 3 the db node.", true),
        (
            hotspot(),
            "Decide whether statement 2 is true or false.",
            false,
        ),
        (hotspot(), "Look at statement 2, no need to rush.", false),
        (hotspot(), "Is statement 1 true or false?", false),
        (hotspot(), "If you ask me, statement 1 is true.", true),
        (single(), "It is Mercury.", true),
        (
            single(),
            "The best thing you'd do is compare orbits.",
            false,
        ),
        (
            single(),
            "The best way, e.g. looking at orbits, is slow.",
            false,
        ),
    ]
}

const PAD: &str =
    "Consider the orbital periods and how much light each body receives over a year. ";
const PAD_H: &str = "Review the cluster state and think about uptime and backups in general. ";

fn pad_for(fx: &Fx) -> &'static str {
    if matches!(fx.0.kind, Kind::Hotspot) {
        PAD_H
    } else {
        PAD
    }
}

#[test]
fn every_two_way_split_gives_the_whole_text_verdict() {
    for (fx, text, expect) in samples() {
        assert_eq!(whole(&fx, text), expect, "whole: {text:?}");
        let idx: Vec<usize> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();
        for &i in &idx {
            let (a, b) = text.split_at(i);
            let (out, leaked) = stream(&fx, &[a, b]);
            assert_eq!(leaked, expect, "split {i} of {text:?}");
            if !leaked {
                assert_eq!(out, text, "split {i} of {text:?}");
            }
        }
    }
}

#[test]
fn every_two_way_byte_split_gives_the_whole_text_verdict() {
    for (fx, text, expect) in samples() {
        for i in 0..=text.len() {
            let mut g = HintGuard::new(&fx.0, &fx.1);
            let mut out = String::new();
            let mut leaked = false;
            for part in [&text.as_bytes()[..i], &text.as_bytes()[i..]] {
                match g.push_bytes(part) {
                    Verdict::Release(s) => out.push_str(&s),
                    Verdict::Leak => leaked = true,
                }
            }
            if !leaked {
                match g.finish() {
                    Verdict::Release(s) => out.push_str(&s),
                    Verdict::Leak => leaked = true,
                }
            }
            assert_eq!(leaked, expect, "byte split {i} of {text:?}");
            if !leaked {
                assert_eq!(out, text, "byte split {i} of {text:?}");
            }
        }
    }
}

#[test]
fn small_chunks_with_padding_before_and_after() {
    for (fx, text, expect) in samples() {
        let pad = pad_for(&fx);
        for lead in [0usize, 3] {
            for pre in [0usize, 1, 2, 5, 13, 21, 31, 32, 33] {
                let full = format!("{}{} {}{}", pad.repeat(lead), "k".repeat(pre), text, pad);
                assert_eq!(whole(&fx, &full), expect, "whole: {full:?}");
                for n in [1usize, 2, 5, 13, 31, 32, 33, 64] {
                    let (out, leaked) = stream_chunks(&fx, &full, n);
                    assert_eq!(
                        leaked, expect,
                        "chunk {n}, pre {pre}, lead {lead}: {text:?}"
                    );
                    if !leaked {
                        assert_eq!(out, full);
                    }
                }
            }
        }
    }
}

#[test]
fn byte_chunks_inside_utf8_sequences() {
    for (fx, text, expect) in samples() {
        let full = format!(
            "{}{}{}",
            "思考一下。".repeat(8),
            text,
            "また考えてみましょう。"
        );
        let want = whole(&fx, &full);
        assert_eq!(want, expect, "{text:?}");
        for n in 1..=7 {
            let (out, leaked) = stream_bytes(&fx, &full, n);
            assert_eq!(leaked, want, "bytes {n}: {text:?}");
            if !leaked {
                assert_eq!(out, full);
            }
        }
    }
}

#[test]
fn a_long_key_text_is_held_back_until_checked() {
    let long: String = (0..40)
        .map(|i| format!("word{i} "))
        .collect::<String>()
        .trim()
        .to_string();
    assert!(long.chars().count() > 200);
    let q = question(Kind::Single, "Which one?", &["First", "Second"]);
    let fx = (q, sol(&[&long]));
    let full = format!("Warm up first. {long} Then finish.");
    assert!(whole(&fx, &full));
    for n in [1usize, 3, 10, 31, 100] {
        let (out, leaked) = stream_chunks(&fx, &full, n);
        assert!(leaked, "chunk {n}");
        // Nothing of the quote was released before the verdict.
        assert!(!out.contains("word20"), "chunk {n}: {out:?}");
    }
    let hold = HintGuard::new(&fx.0, &fx.1).hold();
    assert!(hold >= long.chars().count());
}

#[test]
fn invalid_and_incomplete_bytes_become_replacement_characters() {
    let fx = single();
    let mut g = HintGuard::new(&fx.0, &fx.1);
    let mut out = String::new();
    for part in [&b"ab\xff cd"[..], &b"\xe7\xad"[..]] {
        if let Verdict::Release(s) = g.push_bytes(part) {
            out.push_str(&s);
        }
    }
    match g.finish() {
        Verdict::Release(s) => out.push_str(&s),
        Verdict::Leak => panic!("unexpected leak"),
    }
    assert_eq!(out, "ab\u{FFFD} cd\u{FFFD}");
}

#[test]
fn random_texts_stream_like_whole_texts() {
    let vocab_single: &[&str] = &[
        "The",
        "answer",
        "is",
        "B",
        "c",
        "Mars",
        "Mercury",
        "option",
        "second",
        "wrong",
        "correct",
        "think",
        "about",
        "orbit",
        "distance",
        "(A)",
        "答案",
        "选",
        "第2项",
        "是错的",
        "水星",
        "正解",
        "は",
        "。",
        ".",
        "\n",
        "\n\n",
        "，",
        "the",
        "nearest",
        "planet",
        "to",
        "Sun",
        "a",
        "A",
        "#3",
        "not",
        "isn't",
        "→",
        "**D**",
        "Venus",
        "最後の選択肢",
        "を選んで",
        "the last one",
    ];
    let vocab_hot: &[&str] = &[
        "Statement",
        "1",
        "2",
        "3",
        "is",
        "true",
        "false",
        "yes",
        "No",
        "item",
        "(2)",
        "[3]",
        "web-node",
        "db-node",
        "think",
        "about",
        "uptime",
        "第1项",
        "选",
        "是",
        "否",
        "。",
        "：",
        "1番目",
        "は",
        "正しい",
        "\n",
        "\n\n",
        ",",
        "the",
        "first",
        "statement",
        "and",
        "closely",
    ];
    for (fx, vocab, seed) in [
        (single(), vocab_single, 1u64),
        (multi(), vocab_single, 2),
        (single_class_b(), vocab_single, 3),
        (hotspot(), vocab_hot, 4),
    ] {
        let mut rng = Lcg(seed);
        let mut leaks_seen = 0;
        for case in 0..1500 {
            let tokens = 3 + rng.next(70);
            let mut text = String::new();
            for _ in 0..tokens {
                text.push_str(vocab[rng.next(vocab.len())]);
                text.push_str(["", " ", " ", "  ", "\n"][rng.next(5)]);
            }
            let want = whole(&fx, &text);
            leaks_seen += usize::from(want);
            let mut pieces: Vec<&str> = Vec::new();
            let mut rest = text.as_str();
            while !rest.is_empty() {
                let n = 1 + rng.next(45);
                let cut = rest.char_indices().nth(n).map_or(rest.len(), |(i, _)| i);
                pieces.push(&rest[..cut]);
                rest = &rest[cut..];
            }
            let (out, leaked) = stream(&fx, &pieces);
            assert_eq!(leaked, want, "case {case} (seed {seed}): {text:?}");
            if !leaked {
                assert_eq!(out, text, "case {case} (seed {seed})");
            }
        }
        assert!(
            leaks_seen > 50,
            "the generator must produce leaks ({leaks_seen})"
        );
    }
}
