//! v1.6 bank download, Word side: the .docx writer builds a valid package in the reading order of the contract
//! (title, English, Chinese, Japanese; every sheet; every question with its answer and explanation). Self-made sample
//! questions only; no parser and no store are involved.
use std::io::Read;

use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};
use xexam_backend::export::docx::write_docx;
use xexam_backend::export::{ExportBank, ExportSheet};
use xexam_backend::models::Question;

const NOW: &str = "2026-11-16T08:09:10Z";

fn now() -> jiff::Timestamp {
    NOW.parse().expect("timestamp")
}

/// A question from the interesting fields; the rest gets neutral defaults.
fn question(v: Value) -> Question {
    let mut base = json!({
        "kind": "single", "tag": "", "stem": "", "options": [], "items": [], "answer": [],
        "translation": "", "zh": null, "lang": "en", "ja": null,
        "explanation": "", "mnemonic": "", "note": "",
    });
    for (k, val) in v.as_object().expect("object") {
        base[k] = val.clone();
    }
    serde_json::from_value(base).expect("question")
}

const STEM_SKY: &str = "Which colour is the sky on a clear day?";
const STEM_ESC: &str = "Fruit <b> & \"q\" 'x'\tend\u{1}!";
const STEM_ESC_CLEAN: &str = "Fruit <b> & \"q\" 'x'\tend!";
const STEM_HOT_A: &str = "Decide for each statement.";
const STEM_HOT_B: &str = "Decide again.";
const STEM_PLAIN: &str = "How many days are in a week?";

fn q_single() -> Question {
    question(json!({
        "kind": "single", "stem": STEM_SKY,
        "options": [{"key": "A", "text": "Red"}, {"key": "B", "text": "Blue"}, {"key": "C", "text": "Green"}],
        "answer": ["B"],
        "zh": {"stem": "晴天的天空是什么颜色？", "options": ["红色", "蓝色", "绿色"]},
        "ja": {"stem": "晴れた日の空は何色ですか。", "options": ["赤", "青", "緑"]},
        "explain": {
            "en": {"explanation": "Air scatters blue light.", "mnemonic": "Blue sky, blue light.", "note": "sure"},
            "zh": {"explanation": "空气散射蓝光。", "mnemonic": "蓝天蓝光。", "note": "确定"},
            "ja": {"explanation": "空気が青い光を散乱します。", "mnemonic": "青空は青い光。", "note": "確実"},
        },
    }))
}

fn q_multi() -> Question {
    question(json!({
        "kind": "multi", "stem": STEM_ESC,
        "options": [{"key": "A", "text": "Apple"}, {"key": "B", "text": "Brick"}, {"key": "C", "text": "Cherry"}],
        "answer": ["A", "C"],
        "zh": {"stem": "下面哪些是水果？", "options": ["苹果"]},
        "explain": {"en": {"explanation": "Apples and cherries grow on trees.", "mnemonic": "", "note": ""}},
    }))
}

fn q_hotspot_zh_choices() -> Question {
    question(json!({
        "kind": "hotspot", "stem": STEM_HOT_A,
        "items": [
            {"text": "Water is wet", "choices": ["Yes", "No"]},
            {"text": "Fire is cold", "choices": ["Yes", "No"]},
        ],
        "answer": [0, 1],
        "zh": {"stem": "请判断每一句话。", "items": ["水是湿的", "火是冷的"], "choices": [["是", "否"], ["是", "否"]]},
        "explain": {"en": {"explanation": "Water is wet; fire is hot.", "mnemonic": "", "note": ""}},
    }))
}

fn q_hotspot_no_zh_choices() -> Question {
    question(json!({
        "kind": "hotspot", "stem": STEM_HOT_B,
        "items": [
            {"text": "Ice is cold", "choices": ["True", "False"]},
            {"text": "Sand is liquid", "choices": ["True", "False"]},
        ],
        "answer": [1, 0],
        "zh": {"stem": "再判断一次。", "items": ["冰是冷的", "沙子是液体"]},
        "ja": {"stem": "もう一度判断してください。", "items": ["氷は冷たい", "砂は液体"], "choices": [["正", "誤"], ["正", "誤"]]},
    }))
}

/// No `ja`, no `explain`: the compatibility copies carry the explanation.
fn q_plain() -> Question {
    question(json!({
        "kind": "single", "stem": STEM_PLAIN,
        "options": [{"key": "A", "text": "Five"}, {"key": "B", "text": "Seven"}],
        "answer": ["B"],
        "explanation": "A week has seven days.", "mnemonic": "Seven days, one week.", "note": "easy",
    }))
}

fn sample_bank() -> ExportBank {
    ExportBank {
        id: 7,
        name: "Sample Bank".to_string(),
        sheets: vec![
            ExportSheet {
                name: "Basics".to_string(),
                questions: vec![q_single(), q_multi()],
            },
            ExportSheet {
                name: "Extras".to_string(),
                questions: vec![q_hotspot_zh_choices(), q_hotspot_no_zh_choices(), q_plain()],
            },
        ],
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading the package back

const PARTS: [&str; 9] = [
    "[Content_Types].xml",
    "_rels/.rels",
    "docProps/core.xml",
    "docProps/app.xml",
    "word/document.xml",
    "word/styles.xml",
    "word/settings.xml",
    "word/fontTable.xml",
    "word/_rels/document.xml.rels",
];

fn read_parts(bytes: &[u8]) -> Vec<(String, String)> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip opens");
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).expect("entry");
        let mut s = String::new();
        f.read_to_string(&mut s).expect("utf-8 part");
        out.push((f.name().expect("entry name").into_owned(), s));
    }
    out
}

fn part<'a>(parts: &'a [(String, String)], name: &str) -> &'a str {
    &parts
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("missing part {name}"))
        .1
}

/// Reads an XML part to the end; panics on any error. Returns the number of elements.
fn parse_to_end(xml: &str) -> usize {
    let mut r = Reader::from_str(xml);
    let mut n = 0;
    loop {
        match r.read_event().expect("well-formed XML") {
            Event::Start(_) | Event::Empty(_) => n += 1,
            Event::Eof => return n,
            _ => {}
        }
    }
}

#[derive(Debug, Default, Clone)]
struct R {
    text: String,
    bold: bool,
    lang: String,
}

#[derive(Debug, Default, Clone)]
struct P {
    style: String,
    page_break: bool,
    keep_next: bool,
    runs: Vec<R>,
}

impl P {
    fn text(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
}

fn attr(e: &quick_xml::events::BytesStart, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .map(|a| a.value.to_string())
}

/// The body paragraphs of document.xml: style, flags and runs (text with `<w:tab/>` as TAB and `<w:br/>` as LF).
fn paragraphs(xml: &str) -> Vec<P> {
    let mut r = Reader::from_str(xml);
    let mut out: Vec<P> = Vec::new();
    let mut cur: Option<P> = None;
    let mut run: Option<R> = None;
    let mut in_t = false;
    loop {
        let ev = r.read_event().expect("well-formed XML");
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let name = e.name();
                match name.as_ref() {
                    "w:p" => cur = Some(P::default()),
                    "w:r" => run = Some(R::default()),
                    "w:pStyle" => {
                        if let Some(p) = cur.as_mut() {
                            p.style = attr(e, "w:val").unwrap_or_default();
                        }
                    }
                    "w:pageBreakBefore" => {
                        if let Some(p) = cur.as_mut() {
                            p.page_break = true;
                        }
                    }
                    "w:keepNext" => {
                        if let Some(p) = cur.as_mut() {
                            p.keep_next = true;
                        }
                    }
                    "w:b" => {
                        if let Some(x) = run.as_mut() {
                            x.bold = true;
                        }
                    }
                    "w:lang" => {
                        if let Some(x) = run.as_mut() {
                            x.lang = attr(e, "w:val").unwrap_or_default();
                        }
                    }
                    "w:tab" => {
                        if let Some(x) = run.as_mut() {
                            x.text.push('\t');
                        }
                    }
                    "w:br" => {
                        if let Some(x) = run.as_mut() {
                            x.text.push('\n');
                        }
                    }
                    "w:t" => in_t = matches!(ev, Event::Start(_)),
                    _ => {}
                }
            }
            Event::End(ref e) => match e.name().as_ref() {
                "w:t" => in_t = false,
                "w:r" => {
                    if let (Some(x), Some(p)) = (run.take(), cur.as_mut()) {
                        p.runs.push(x);
                    }
                }
                "w:p" => out.extend(cur.take()),
                _ => {}
            },
            Event::Text(ref t) if in_t => {
                if let Some(x) = run.as_mut() {
                    x.text.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(ref g) if in_t => {
                let s = if g.is_char_ref() {
                    g.resolve_char_ref()
                        .expect("char ref")
                        .expect("char")
                        .to_string()
                } else {
                    match &**g {
                        "lt" => "<",
                        "gt" => ">",
                        "amp" => "&",
                        "apos" => "'",
                        "quot" => "\"",
                        other => panic!("unexpected entity {other}"),
                    }
                    .to_string()
                };
                if let Some(x) = run.as_mut() {
                    x.text.push_str(&s);
                }
            }
            Event::Eof => return out,
            _ => {}
        }
    }
}

fn build(bank: &ExportBank) -> (Vec<(String, String)>, Vec<P>) {
    let bytes = write_docx(bank, now()).expect("docx");
    let parts = read_parts(&bytes);
    let paras = paragraphs(part(&parts, "word/document.xml"));
    (parts, paras)
}

/// The paragraphs of one language section (1 = English, 2 = Chinese, 3 = Japanese), heading included.
fn section(paras: &[P], n: usize) -> Vec<P> {
    let mut idx = 0;
    let mut out = Vec::new();
    for p in paras {
        if p.style == "Heading1" {
            idx += 1;
        }
        if idx == n {
            out.push(p.clone());
        }
    }
    out
}

fn find<'a>(ps: &'a [P], text: &str) -> &'a P {
    ps.iter()
        .find(|p| p.text() == text)
        .unwrap_or_else(|| panic!("no paragraph {text:?}"))
}

fn sheet_names_in(ps: &[P]) -> Vec<String> {
    ps.iter()
        .filter(|p| p.style == "Heading2")
        .map(P::text)
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// Tests

#[test]
fn package_is_valid_and_complete() {
    let (parts, _) = build(&sample_bank());
    let names: Vec<&str> = parts.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, PARTS, "all nine parts, in the contract order");
    for (name, xml) in &parts {
        assert!(parse_to_end(xml) > 0, "{name} parses to the end");
        assert!(xml.starts_with("<?xml"), "{name} has an XML declaration");
    }
    let ct = part(&parts, "[Content_Types].xml");
    for (name, _) in &parts {
        if name != "[Content_Types].xml"
            && name != "_rels/.rels"
            && name != "word/_rels/document.xml.rels"
        {
            assert!(
                ct.contains(&format!("PartName=\"/{name}\"")),
                "override for {name}"
            );
        }
    }
    assert!(ct.contains("Extension=\"rels\"") && ct.contains("Extension=\"xml\""));
    let core = part(&parts, "docProps/core.xml");
    assert!(core.contains("<dc:title>Sample Bank</dc:title>"));
    assert!(core.contains("<dc:creator>Xexam</dc:creator>"));
    assert!(
        core.contains(">2026-11-16T08:09:10Z<"),
        "creation time comes from the argument"
    );
    assert!(core.contains("xsi:type=\"dcterms:W3CDTF\""));
    let styles = part(&parts, "word/styles.xml");
    assert!(styles.contains("w:val=\"heading 1\"") && styles.contains("w:val=\"heading 2\""));
    assert!(part(&parts, "word/settings.xml").contains("compatibilityMode"));
    assert!(part(&parts, "word/_rels/document.xml.rels").contains("Target=\"fontTable.xml\""));
}

#[test]
fn reading_order_headings_and_numbering() {
    let (_, paras) = build(&sample_bank());
    assert_eq!(paras[0].style, "Title");
    assert_eq!(paras[0].text(), "Sample Bank");
    let h1: Vec<&P> = paras.iter().filter(|p| p.style == "Heading1").collect();
    assert_eq!(
        h1.iter().map(|p| p.text()).collect::<Vec<_>>(),
        ["English", "中文", "日本語"]
    );
    assert!(!h1[0].page_break, "English follows the title");
    assert!(
        h1[1].page_break && h1[2].page_break,
        "each later language starts a new page"
    );
    for n in 1..=3 {
        let s = section(&paras, n);
        assert_eq!(sheet_names_in(&s), ["Basics", "Extras"]);
        // numbering restarts at 1 in every sheet: number lines are the keepNext paragraphs starting with "<n>. "
        let mut numbers: Vec<Vec<String>> = Vec::new();
        for p in &s {
            if p.style == "Heading2" {
                numbers.push(Vec::new());
            } else if p.keep_next
                && p.runs
                    .first()
                    .is_some_and(|r| r.bold && r.text.ends_with(". "))
            {
                numbers
                    .last_mut()
                    .expect("sheet")
                    .push(p.runs[0].text.trim().to_string());
            }
        }
        assert_eq!(numbers, vec![vec!["1.", "2."], vec!["1.", "2.", "3."]]);
    }
    let en = section(&paras, 1);
    assert!(en.iter().any(|p| p.text() == "1. [Single choice]"));
    assert!(en.iter().any(|p| p.text() == "2. [Multiple choice]"));
    assert!(en.iter().any(|p| p.text() == "1. [Hotspot]"));
    assert!(section(&paras, 2)
        .iter()
        .any(|p| p.text() == "2. ［多选题］"));
    assert!(section(&paras, 3)
        .iter()
        .any(|p| p.text() == "1. ［ホットスポット］"));
}

#[test]
fn correct_answers_are_marked() {
    let (_, paras) = build(&sample_bank());
    let en = section(&paras, 1);
    for (text, correct) in [("A. Apple", true), ("B. Brick", false), ("C. Cherry", true)] {
        let p = en
            .iter()
            .find(|p| p.text().starts_with(text))
            .unwrap_or_else(|| panic!("option {text}"));
        assert_eq!(p.text().contains('\u{2713}'), correct, "{text}");
        assert_eq!(p.runs.iter().all(|r| r.bold), correct, "{text}");
        assert_eq!(p.runs[0].bold, correct, "{text}");
    }
    // Hotspot: the stored index of each item is marked.
    assert_eq!(
        find(&en, "Yes \u{2713} / No")
            .runs
            .iter()
            .filter(|r| r.bold)
            .count(),
        2
    );
    find(&en, "Yes / No \u{2713}");
    find(&en, "Answer: 1. Yes; 2. No");
    find(&en, "Answer: A, C");
    find(&en, "Answer: B");
    // Chinese section uses the Chinese choices where they exist; the other hotspot falls back to English choices.
    let zh = section(&paras, 2);
    find(&zh, "是 \u{2713} / 否");
    find(&zh, "是 / 否 \u{2713}");
    find(&zh, "答案：1. 是; 2. 否");
    find(&zh, "True / False \u{2713}");
    // Japanese: choices exist for the second hotspot only.
    let ja = section(&paras, 3);
    find(&ja, "正 / 誤 \u{2713}");
    find(&ja, "正 \u{2713} / 誤");
    find(&ja, "Yes \u{2713} / No");
}

#[test]
fn languages_and_fallbacks() {
    let (_, paras) = build(&sample_bank());
    let zh = section(&paras, 2);
    let stem = find(&zh, "晴天的天空是什么颜色？");
    assert!(stem.runs.iter().all(|r| r.lang == "zh-CN"));
    find(&zh, "B. 蓝色 \u{2713}");
    let ja = section(&paras, 3);
    let stem = find(&ja, "晴れた日の空は何色ですか。");
    assert!(stem.runs.iter().all(|r| r.lang == "ja-JP"));
    assert!(find(&ja, "解説：空気が青い光を散乱します。")
        .runs
        .iter()
        .all(|r| r.lang == "ja-JP"));
    // zh has one option only: the others are English text marked en-US
    let b = find(&zh, "A. 苹果 \u{2713}");
    assert_eq!(b.runs[0].lang, "zh-CN");
    let brick = find(&zh, "B. Brick");
    assert!(brick.runs.iter().all(|r| r.lang == "en-US"));
    // ja: None -> the English stem, marked en-US
    let fallback = ja
        .iter()
        .find(|p| p.text() == STEM_ESC_CLEAN)
        .expect("English stem in the ja part");
    assert!(fallback.runs.iter().all(|r| r.lang == "en-US"));
    // the explanation falls back to English when explain.ja is missing
    let ex = find(&ja, "解説：Apples and cherries grow on trees.");
    assert_eq!(
        ex.runs[0].lang, "ja-JP",
        "the label stays in the section language"
    );
    assert_eq!(ex.runs[1].lang, "en-US");
    // no explain at all: the compatibility copies, with note and mnemonic
    find(&zh, "解析：A week has seven days.");
    find(&zh, "记忆口诀：Seven days, one week.");
    find(&zh, "备注：easy");
    // note and mnemonic of the single question
    let en = section(&paras, 1);
    find(&en, "Memory aid: Blue sky, blue light.");
    find(&en, "Note: sure");
    find(&ja, "覚え方：青空は青い光。");
    find(&ja, "備考：確実");
}

#[test]
fn escaping_tab_and_control_characters() {
    let bytes = write_docx(&sample_bank(), now()).expect("docx");
    let parts = read_parts(&bytes);
    let xml = part(&parts, "word/document.xml");
    assert!(xml.contains("&lt;b&gt; &amp; &quot;q&quot; &apos;x&apos;"));
    assert!(xml.contains("<w:tab/>"));
    assert!(!xml.contains('\u{1}'));
    let paras = paragraphs(xml);
    let en = section(&paras, 1);
    let p = en
        .iter()
        .find(|p| p.text() == STEM_ESC_CLEAN)
        .expect("escaped stem round-trips as text");
    assert!(p.text().contains('\t'));
    // a title with markup characters and a CJK title
    let mut bank = sample_bank();
    bank.name = "示例 <题库> & more".to_string();
    let (parts, paras) = build(&bank);
    assert_eq!(paras[0].text(), "示例 <题库> & more");
    assert_eq!(paras[0].runs[0].lang, "zh-CN");
    assert!(part(&parts, "docProps/core.xml").contains("&lt;题库&gt; &amp; more"));
    bank.name = "サンプル".to_string();
    assert_eq!(build(&bank).1[0].runs[0].lang, "ja-JP");
}

#[test]
fn line_breaks_become_w_br() {
    let q = question(json!({
        "kind": "single", "stem": "Line one\nLine two",
        "options": [{"key": "A", "text": "x"}], "answer": ["A"],
    }));
    let bank = ExportBank {
        id: 1,
        name: "B".to_string(),
        sheets: vec![ExportSheet {
            name: "S".to_string(),
            questions: vec![q],
        }],
    };
    let bytes = write_docx(&bank, now()).expect("docx");
    let parts = read_parts(&bytes);
    let xml = part(&parts, "word/document.xml");
    assert!(xml.contains("Line one</w:t><w:br/><w:t xml:space=\"preserve\">Line two"));
    // every text element keeps its spaces
    assert!(!xml.contains("<w:t>"));
}

#[test]
fn empty_bank_is_a_valid_package() {
    let bank = ExportBank {
        id: 1,
        name: "Empty".to_string(),
        sheets: vec![],
    };
    let (parts, paras) = build(&bank);
    for (name, xml) in &parts {
        parse_to_end(xml);
        assert!(!name.is_empty());
    }
    let h1: Vec<String> = paras
        .iter()
        .filter(|p| p.style == "Heading1")
        .map(P::text)
        .collect();
    assert_eq!(h1, ["English", "中文", "日本語"]);
    for line in ["(no questions)", "（没有题目）", "（問題がありません）"] {
        find(&paras, line);
    }
    assert!(paras.iter().all(|p| p.style != "Heading2"));
}

#[test]
fn defensive_against_bad_data() {
    let bad_hotspot = question(json!({
        "kind": "hotspot", "stem": "Bad indexes",
        "items": [{"text": "one", "choices": ["Yes", "No"]}, {"text": "two", "choices": ["Yes", "No"]}],
        "answer": [5, -1, 0],
        "zh": {"stem": "坏索引", "items": ["一"], "choices": [["是"]]},
    }));
    let short_zh = question(json!({
        "kind": "multi", "stem": "Short zh",
        "options": [{"key": "A", "text": "a"}, {"key": "B", "text": "b"}, {"key": "C", "text": "c"}],
        "answer": ["A", "Z"],
        "zh": {"stem": "", "options": ["甲"]},
        "ja": {"stem": "短い", "options": []},
    }));
    let wrong_shape = question(json!({
        "kind": "single", "stem": "Letters expected",
        "options": [{"key": "A", "text": "a"}], "answer": [1, 2],
    }));
    let wrong_shape2 =
        question(json!({"kind": "hotspot", "stem": "Indexes expected", "answer": ["A"]}));
    let bank = ExportBank {
        id: 1,
        name: "Bad".to_string(),
        sheets: vec![ExportSheet {
            name: "S".to_string(),
            questions: vec![bad_hotspot, short_zh, wrong_shape, wrong_shape2],
        }],
    };
    let (parts, paras) = build(&bank);
    parse_to_end(part(&parts, "word/document.xml"));
    let en = section(&paras, 1);
    assert!(en.iter().any(|p| p.text() == "Answer: 1. ?; 2. ?; 3. ?"));
    assert!(en
        .iter()
        .all(|p| !p.text().starts_with("Yes") || !p.text().contains('\u{2713}')));
    // the zh struct has one option and an empty stem: English fills the gaps
    let zh = section(&paras, 2);
    find(&zh, "Short zh");
    find(&zh, "A. 甲 \u{2713}");
    find(&zh, "B. b");
    // unknown answer letters are simply not matched
    assert!(en.iter().any(|p| p.text() == "Answer: A, Z"));
    assert!(en.iter().filter(|p| p.text() == "Answer: ?").count() >= 2);
}

/// Writes the sample bank where the coordinator can open it: set `XEXAM_SAMPLE_OUT` to a file path.
#[test]
fn write_sample_file_when_asked() {
    let Ok(path) = std::env::var("XEXAM_SAMPLE_OUT") else {
        return;
    };
    let bytes = write_docx(&sample_bank(), now()).expect("docx");
    std::fs::write(&path, &bytes).expect("write sample");
    for (_, xml) in read_parts(&bytes) {
        parse_to_end(&xml);
    }
}

/// Dev only: writes the .docx and .xlsx of one bank of a real data directory, to look at in Word / LibreOffice / Google
/// Docs / iPhone Files. Never run in CI. The output must stay outside the repository.
#[test]
#[ignore]
fn write_files_of_a_local_bank() {
    use rusqlite::{Connection, OpenFlags};
    let (Ok(data), Ok(bank_id)) = (
        std::env::var("XEXAM_EXPORT_DATA"),
        std::env::var("XEXAM_EXPORT_BANK"),
    ) else {
        println!(
            "usage: XEXAM_EXPORT_DATA=<data dir> XEXAM_EXPORT_BANK=<bank id> [XEXAM_EXPORT_OUT=<out dir>] \
             cargo test --test v16_docx -- --ignored --nocapture write_files_of_a_local_bank"
        );
        return;
    };
    let id: i64 = bank_id.parse().expect("XEXAM_EXPORT_BANK is a number");
    let out_dir = std::env::var("XEXAM_EXPORT_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let db = Connection::open_with_flags(
        std::path::Path::new(&data).join("xexam.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .expect("open database read-only");
    let name: String = db
        .query_row("SELECT name FROM banks WHERE id=?1", [id], |r| r.get(0))
        .expect("bank exists");
    let mut stmt = db
        .prepare(
            "SELECT sheet, row_no, data FROM questions WHERE bank_id=?1 AND active=1 ORDER BY seq",
        )
        .expect("prepare");
    let rows = stmt
        .query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .expect("query");
    let mut sheets: Vec<ExportSheet> = Vec::new();
    for row in rows {
        let (sheet, row_no, data) = row.expect("row");
        let mut q: Question = serde_json::from_str(&data).expect("question data");
        q.sheet = sheet.clone();
        q.row = row_no;
        match sheets.iter_mut().find(|s| s.name == sheet) {
            Some(s) => s.questions.push(q),
            None => sheets.push(ExportSheet {
                name: sheet,
                questions: vec![q],
            }),
        }
    }
    let bank = ExportBank { id, name, sheets };
    std::fs::create_dir_all(&out_dir).expect("output directory");
    for format in [
        xexam_backend::export::ExportFormat::Docx,
        xexam_backend::export::ExportFormat::Xlsx,
    ] {
        let bytes =
            xexam_backend::export::export(&bank, format, jiff::Timestamp::now()).expect("export");
        let path = out_dir.join(format!("bank-{id}.{}", format.as_str()));
        std::fs::write(&path, &bytes).expect("write file");
        println!(
            "{}: {} sheets, {} questions, {} bytes",
            path.display(),
            bank.sheets.len(),
            bank.question_count(),
            bytes.len()
        );
    }
}
