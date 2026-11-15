//! v1.3 contract test, ai-service half: the golden file `tests/fixtures/gen_corpus_page.json` is what the real backend answers
//! on `POST /internal/ai/gen/corpus` (the backend's `tests/v13_contract.rs` compares its live answer with the same file).
//! Here the file goes through `parse_corpus`, so the answer format (0-based answer positions, absent language keys,
//! first-page extras) is checked on both sides against one file. Made-up kitchen questions only.
use std::path::Path;
use xexam_ai_service::gen::wire::parse_corpus;
use xexam_ai_service::gen::Kind;

fn golden() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/gen_corpus_page.json");
    std::fs::read(path).expect("golden corpus file")
}

#[test]
fn the_backend_corpus_answer_is_read_by_parse_corpus() {
    let page = parse_corpus(&golden()).expect("the golden answer parses");
    assert!(page.user_id > 0);
    assert_eq!(page.total, 7);
    assert_eq!(page.next, None, "one page");
    assert_eq!(page.items.len(), 7);
    assert_eq!(page.banks.len(), 1);
    assert_eq!(page.banks[0].1, "Kitchen basics");
    assert_eq!(page.strata.len(), 1);
    assert!(page.previous.is_empty());
    // source order, ids kept
    assert_eq!(page.items[0].id, "ov001");
    assert_eq!(page.items[6].id, "noja1");
    // all three languages of the first question, answer = 0-based positions
    let first = &page.items[0];
    assert_eq!(first.sheet, "Ovens");
    assert_eq!(first.kind, Kind::Single);
    assert_eq!(first.answer, vec![1u8]);
    assert_eq!(first.stem.en, "Which oven is number 1?");
    assert!(first.stem.zh.ends_with("中文"));
    assert!(first.stem.ja.ends_with("日本語"));
    assert_eq!(first.options.len(), 3);
    assert_eq!(first.options[1].en, "Ladle");
    assert_eq!(first.options[1].zh, "汤勺");
    assert_eq!(first.options[1].ja, "おたま");
    // an explanation the question lacks in one language is absent in the file and empty here
    assert_eq!(first.explanation.en, "A ladle serves soup.");
    assert_eq!(first.explanation.zh, "汤勺用来盛汤。");
    assert_eq!(first.explanation.ja, "");
    // the multiple-choice question
    assert_eq!(page.items[3].kind, Kind::Multi);
    assert_eq!(page.items[3].answer, vec![0u8, 1u8]);
    // a question without a Japanese text: the key is absent in the file, empty here, never a failure
    let plain = &page.items[6];
    assert_eq!(plain.stem.ja, "");
    assert!(plain.options.iter().all(|o| o.ja.is_empty()));
    assert_eq!(plain.options[0].zh, "打蛋器");
}

#[test]
fn a_changed_answer_format_is_refused_not_guessed() {
    // an answer position outside the options, a missing user and a negative total are unreadable
    let mut v: serde_json::Value = serde_json::from_slice(&golden()).expect("json");
    v["items"][0]["answer"] = serde_json::json!([9]);
    assert!(parse_corpus(v.to_string().as_bytes()).is_err());
    let mut v: serde_json::Value = serde_json::from_slice(&golden()).expect("json");
    v["user"] = serde_json::json!({});
    assert!(parse_corpus(v.to_string().as_bytes()).is_err());
    let mut v: serde_json::Value = serde_json::from_slice(&golden()).expect("json");
    v["total"] = serde_json::json!(-1);
    assert!(parse_corpus(v.to_string().as_bytes()).is_err());
}
