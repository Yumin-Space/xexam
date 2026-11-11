//! The six-column layout (v0.7): A English, B Chinese, C Japanese questions; D English, E Chinese, F Japanese answers,
//! explanations and memory aids. English is authoritative: type, options / items, id and answer key come from A and D.
//! Builds the xlsx in memory with made-up questions only; does not depend on any real question bank.
use std::io::{Cursor, Write};

use serde_json::Value;
use xexam_backend::database::Store;
use xexam_backend::models::{Answer, Kind, ParsedWorkbook, Question};
use xexam_backend::parser::{check_xlsx, make_id, parse_workbook};
use xexam_backend::services::preview_payload;

const WB: &str = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="worksheets/a.xml"/></Relationships>"#;

const SKIP6_NO_ENGLISH: &str = "这一行没有英文题目（六列格式以 A 列的英文题目为准）";
const SKIP6_NO_ANSWER: &str = "D 列没有找到英文答案（D 列要写「【Answer】…」）";
const WARN6_ZH_DIFFERS: &str = "E 列的中文答案和 D 列的英文答案不一致（已按英文答案导入）";
const WARN6_JA_DIFFERS: &str = "F 列的日文答案和 D 列的英文答案不一致（已按英文答案导入）";

fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (n, b) in parts {
        w.start_file(*n, opts).expect("start");
        w.write_all(b).expect("write");
    }
    let mut out = w.finish().expect("finish").into_inner();
    if out.len() < 100 {
        out.resize(100, 0);
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Each row is a list of cell texts (None = the cell is empty); columns start at A.
fn workbook(rows: &[Vec<Option<&str>>]) -> Vec<u8> {
    let mut sheet = String::new();
    for (i, cols) in rows.iter().enumerate() {
        let r = i + 1;
        sheet.push_str(&format!(r#"<row r="{r}">"#));
        for (k, text) in cols.iter().enumerate() {
            if let Some(t) = text {
                let letter = (b'A' + u8::try_from(k).expect("col")) as char;
                sheet.push_str(&format!(
                    r#"<c r="{letter}{r}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                    esc(t)
                ));
            }
        }
        sheet.push_str("</row>");
    }
    let xml = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{sheet}</sheetData></worksheet>"#
    );
    let data = zip_of(&[
        ("xl/workbook.xml", WB.as_bytes()),
        ("xl/_rels/workbook.xml.rels", RELS.as_bytes()),
        ("xl/worksheets/a.xml", xml.as_bytes()),
    ]);
    check_xlsx(&data).expect("valid xlsx");
    data
}

fn parse(rows: &[Vec<Option<&str>>]) -> ParsedWorkbook {
    parse_workbook(&workbook(rows)).expect("parse")
}

fn six(cells: [&str; 6]) -> Vec<Option<&str>> {
    cells
        .iter()
        .map(|c| (!c.is_empty()).then_some(*c))
        .collect()
}

fn only(pw: &ParsedWorkbook) -> &Question {
    assert_eq!(pw.questions.len(), 1, "sheets: {:?}", pw.sheets);
    &pw.questions[0]
}

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ------------------------------------------------------------------ made-up fixtures

// A multiple-choice question about a made-up garden, with answer notes in parentheses in all three answer columns
const EN_CHOICE: &str = "A gardener waters a small herb garden every morning. Which TWO plants need the most sun? (Select TWO.)\n\nA. Mint\n\nB. Basil\n\nC. Rosemary\n\nD. Parsley";
const ZH_CHOICE: &str = "【中文翻译】\n一位园丁每天早上给小香草园浇水。哪两种植物最需要阳光？（选两项）\nA. 薄荷\nB. 罗勒\nC. 迷迭香\nD. 欧芹";
const JA_CHOICE: &str = "庭師が毎朝小さなハーブ園に水をやっています。最も日光を必要とする植物はどれですか。（2つ選択）\n\nA. ミント\n\nB. バジル\n\nC. ローズマリー\n\nD. パセリ";
const EN_CHOICE_ANS: &str = "【Answer】B, C (confidence: high)\n\n【Explanation】\nBasil and rosemary come from sunny regions.\n\n【Memory aid】Sunny herbs: basil and rosemary.";
const ZH_CHOICE_ANS: &str =
    "【答案】B、C（把握度：高）\n\n【解析·怎么选】\n罗勒和迷迭香来自阳光充足的地区。\n\n【记忆口诀】晒太阳：罗勒、迷迭香。";
const JA_CHOICE_ANS: &str = "【正解】B、C（確信度：高）\n\n【解説】\nバジルとローズマリーは日当たりの良い地域の植物です。\n\n【覚え方】日なたのハーブ：バジルとローズマリー。";

// A hotspot question with category labels: English, Chinese and Japanese labels in the questions; the Chinese answer
// uses the English labels (as some real files do) and the Japanese answer its own labels
const EN_HOT_CAT: &str = "HOTSPOT\nA bakery plans its morning. For each task, select the stage it belongs to.\n\n1. Knead the dough.\u{3000}Prepare / Bake / Sell\n\n2. Put the loaves in the oven.\u{3000}Prepare / Bake / Sell\n\n3. Hand the bread to a customer.\u{3000}Prepare / Bake / Sell";
const ZH_HOT_CAT: &str = "【中文翻译】\n热点题：一家面包店安排早上的工作。请为每项任务选择所属的阶段。\n1. 揉面团。\t准备 / 烘焙 / 销售\n2. 把面包放进烤箱。\t准备 / 烘焙 / 销售\n3. 把面包交给顾客。\t准备 / 烘焙 / 销售";
const JA_HOT_CAT: &str = "ホットスポット\nパン屋が朝の作業を計画しています。各作業について、該当する段階を選択してください。\n\n1. 生地をこねる。\u{3000}準備 / 焼成 / 販売\n\n2. パンをオーブンに入れる。\u{3000}準備 / 焼成 / 販売\n\n3. パンをお客様に渡す。\u{3000}準備 / 焼成 / 販売";
const EN_HOT_CAT_ANS: &str = "【Answer】1 Prepare; 2 Bake; 3 Sell (confidence on item 3: moderate)\n\n【Explanation】\nDough comes first, the oven second, the counter last.\n\n【Memory aid】Knead, bake, sell.";
const ZH_HOT_CAT_ANS: &str = "【答案】1 Prepare；2 Bake；3 Sell（第3项把握度：中等）\n\n【解析·怎么选】\n先揉面，再烘焙，最后销售。\n\n【记忆口诀】揉、烤、卖。";
const JA_HOT_CAT_ANS: &str = "【正解】1 準備；2 焼成；3 販売（第3項の確信度：中程度）\n\n【解説】\n生地、オーブン、カウンターの順です。\n\n【覚え方】こねる、焼く、売る。";

// A yes / no hotspot: 是 / 否 in Chinese, はい / いいえ in Japanese (separated with "、" here)
const EN_HOT_YN: &str = "HOTSPOT\nFor each statement about a square, select Yes if it is true; otherwise, select No.\n\n1. A square has four sides.\tYes / No\n\n2. A square has three corners.\tYes / No";
const ZH_HOT_YN: &str = "【中文翻译】\n热点题：对每条关于正方形的陈述，正确选“是”，否则选“否”。\n1. 正方形有四条边。\t是 / 否\n2. 正方形有三个角。\t是 / 否";
const JA_HOT_YN: &str = "ホットスポット\n正方形に関する各記述について、正しければ「はい」、そうでなければ「いいえ」を選択してください。\n\n1. 正方形には辺が4本ある。\tはい / いいえ\n\n2. 正方形には角が3つある。\tはい / いいえ";
const EN_HOT_YN_ANS: &str =
    "【Answer】1 Yes; 2 No\n\n【Explanation】\nA square has four sides and four corners.";
const ZH_HOT_YN_ANS: &str = "【答案】1 是；2 否\n\n【解析·怎么选】\n正方形有四条边、四个角。";
const JA_HOT_YN_ANS: &str =
    "【正解】1 はい、2 いいえ\n\n【解説】\n正方形には辺が4本、角が4つあります。";

fn choice_row() -> Vec<Option<&'static str>> {
    six([
        EN_CHOICE,
        ZH_CHOICE,
        JA_CHOICE,
        EN_CHOICE_ANS,
        ZH_CHOICE_ANS,
        JA_CHOICE_ANS,
    ])
}

// ------------------------------------------------------------------ tests

#[test]
fn choice_question_is_english_based_with_three_languages() {
    let pw = parse(&[choice_row()]);
    let q = only(&pw);
    assert_eq!(q.kind, Kind::Multi);
    // The note in parentheses is not part of the answer
    assert_eq!(q.answer, Answer::Letters(strings(&["B", "C"])));
    assert_eq!(q.lang, "en");
    assert_eq!(q.tag, "");
    assert!(q.stem.starts_with("A gardener waters"));
    assert_eq!(q.options.len(), 4);
    assert_eq!(q.options[1].text, "Basil");
    // The id is the English identity: sheet + English stem + first English option
    assert_eq!(q.id, make_id("S", &q.stem, "Mint"));
    // The base text is English: no separate `english` / `en`
    assert!(q.english.is_empty() && q.en.is_none());
    let zh = q.zh.as_ref().expect("zh lines up");
    assert!(zh.stem.starts_with("一位园丁"));
    assert_eq!(
        zh.options.as_deref(),
        Some(&strings(&["薄荷", "罗勒", "迷迭香", "欧芹"])[..])
    );
    let ja = q.ja.as_ref().expect("ja lines up");
    assert!(ja.stem.starts_with("庭師が"));
    assert_eq!(
        ja.options.as_deref(),
        Some(&strings(&["ミント", "バジル", "ローズマリー", "パセリ"])[..])
    );
    // Every language keeps its own explanation, memory aid and note
    let ex = q.explain.as_ref().expect("explain");
    let en = ex.en.as_ref().expect("en");
    assert_eq!(
        en.explanation,
        "Basil and rosemary come from sunny regions."
    );
    assert_eq!(en.mnemonic, "Sunny herbs: basil and rosemary.");
    assert_eq!(en.note, "confidence: high");
    let zh_ex = ex.zh.as_ref().expect("zh");
    assert_eq!(zh_ex.explanation, "罗勒和迷迭香来自阳光充足的地区。");
    assert_eq!(zh_ex.note, "把握度：高");
    let ja_ex = ex.ja.as_ref().expect("ja");
    assert_eq!(ja_ex.mnemonic, "日なたのハーブ：バジルとローズマリー。");
    assert_eq!(ja_ex.note, "確信度：高");
    // Compatibility fields: the Chinese texts
    assert_eq!(q.explanation, zh_ex.explanation);
    assert_eq!(q.mnemonic, zh_ex.mnemonic);
    assert_eq!(q.note, "把握度：高");
    // The Chinese question text (without its heading) stays in `translation`
    assert!(q.translation.starts_with("一位园丁"));
    assert!(pw.sheets[0].warnings.is_empty());
    assert_eq!((pw.sheets[0].total, pw.sheets[0].usable), (1, 1));
}

#[test]
fn six_column_question_json_has_its_keys_in_order() {
    let pw = parse(&[choice_row()]);
    let v = serde_json::to_value(only(&pw)).expect("json");
    let keys: Vec<&str> = v
        .as_object()
        .expect("obj")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "kind",
            "tag",
            "stem",
            "options",
            "items",
            "answer",
            "translation",
            "zh",
            "lang",
            "ja",
            "explanation",
            "mnemonic",
            "note",
            "explain",
            "id",
            "sheet",
            "row"
        ]
    );
    let ex: Vec<&str> = v["explain"]
        .as_object()
        .expect("explain")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(ex, ["en", "zh", "ja"]);
    let en: Vec<&str> = v["explain"]["en"]
        .as_object()
        .expect("explain.en")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(en, ["explanation", "mnemonic", "note"]);
    assert_eq!(v["lang"], "en");
    assert_eq!(v["ja"]["options"][0], "ミント");
}

#[test]
fn hotspot_with_category_labels_in_each_language_has_no_warning() {
    let pw = parse(&[six([
        EN_HOT_CAT,
        ZH_HOT_CAT,
        JA_HOT_CAT,
        EN_HOT_CAT_ANS,
        ZH_HOT_CAT_ANS,
        JA_HOT_CAT_ANS,
    ])]);
    let q = only(&pw);
    assert_eq!(q.kind, Kind::Hotspot);
    assert_eq!(q.tag, "HOTSPOT");
    assert_eq!(q.answer, Answer::Indexes(vec![0, 1, 2]));
    assert_eq!(q.items.len(), 3);
    assert_eq!(q.items[0].text, "Knead the dough.");
    assert_eq!(q.items[0].choices, strings(&["Prepare", "Bake", "Sell"]));
    assert!(q.stem.starts_with("A bakery plans"));
    assert_eq!(q.id, make_id("S", &q.stem, "Knead the dough."));
    let zh = q.zh.as_ref().expect("zh");
    assert_eq!(zh.items.as_ref().map(Vec::len), Some(3));
    assert_eq!(
        zh.choices.as_ref().and_then(|c| c.first()).cloned(),
        Some(strings(&["准备", "烘焙", "销售"]))
    );
    let ja = q.ja.as_ref().expect("ja");
    assert!(ja.stem.starts_with("パン屋が"));
    assert_eq!(
        ja.items
            .as_ref()
            .and_then(|i| i.first())
            .map(String::as_str),
        Some("生地をこねる。")
    );
    assert_eq!(
        ja.choices.as_ref().and_then(|c| c.get(2)).cloned(),
        Some(strings(&["準備", "焼成", "販売"]))
    );
    // E answers with the English labels, F with the Japanese ones: both mean the same as D
    assert!(
        pw.sheets[0].warnings.is_empty(),
        "{:?}",
        pw.sheets[0].warnings
    );
    let ex = q.explain.as_ref().expect("explain");
    assert_eq!(
        ex.en.as_ref().map(|t| t.note.as_str()),
        Some("confidence on item 3: moderate")
    );
    assert_eq!(
        ex.ja.as_ref().map(|t| t.mnemonic.as_str()),
        Some("こねる、焼く、売る。")
    );
}

#[test]
fn yes_no_hotspot_matches_in_every_language() {
    let pw = parse(&[six([
        EN_HOT_YN,
        ZH_HOT_YN,
        JA_HOT_YN,
        EN_HOT_YN_ANS,
        ZH_HOT_YN_ANS,
        JA_HOT_YN_ANS,
    ])]);
    let q = only(&pw);
    assert_eq!(q.answer, Answer::Indexes(vec![0, 1]));
    assert_eq!(q.items[1].choices, strings(&["Yes", "No"]));
    assert_eq!(
        q.ja.as_ref().and_then(|j| j.choices.clone()),
        Some(vec![
            strings(&["はい", "いいえ"]),
            strings(&["はい", "いいえ"])
        ])
    );
    assert!(
        pw.sheets[0].warnings.is_empty(),
        "{:?}",
        pw.sheets[0].warnings
    );
    // No memory aid anywhere: the key is still there, empty
    let ex = q.explain.as_ref().expect("explain");
    assert_eq!(ex.zh.as_ref().map(|t| t.mnemonic.as_str()), Some(""));
    assert_eq!(q.mnemonic, "");
}

#[test]
fn a_different_answer_in_e_or_f_is_a_warning_not_an_unreadable_row() {
    let zh_wrong = "【答案】B、D\n\n【解析·怎么选】说明。";
    let ja_unmappable = "【正解】わからない\n\n【解説】説明。";
    let hot_zh_wrong = "【答案】1 是；2 是";
    let pw = parse(&[
        six([
            EN_CHOICE,
            ZH_CHOICE,
            JA_CHOICE,
            EN_CHOICE_ANS,
            zh_wrong,
            ja_unmappable,
        ]),
        six([
            EN_HOT_YN,
            ZH_HOT_YN,
            JA_HOT_YN,
            EN_HOT_YN_ANS,
            hot_zh_wrong,
            JA_HOT_YN_ANS,
        ]),
    ]);
    assert_eq!(pw.questions.len(), 2);
    // Imported with the English answer
    assert_eq!(
        pw.questions[0].answer,
        Answer::Letters(strings(&["B", "C"]))
    );
    assert_eq!(pw.questions[1].answer, Answer::Indexes(vec![0, 1]));
    let s = &pw.sheets[0];
    assert!(s.skipped.is_empty());
    let warned: Vec<(i64, &str)> = s
        .warnings
        .iter()
        .map(|w| (w.row, w.reason.as_str()))
        .collect();
    assert_eq!(
        warned,
        [
            (1, WARN6_ZH_DIFFERS),
            (1, WARN6_JA_DIFFERS),
            (2, WARN6_ZH_DIFFERS)
        ]
    );
}

#[test]
fn rows_without_english_question_or_answer_are_unreadable() {
    let pw = parse(&[
        // Japanese question and answers, but column A is empty
        six([
            "",
            ZH_CHOICE,
            JA_CHOICE,
            EN_CHOICE_ANS,
            ZH_CHOICE_ANS,
            JA_CHOICE_ANS,
        ]),
        // English question, but column D is empty (E and F alone are not enough)
        six([
            EN_CHOICE,
            ZH_CHOICE,
            JA_CHOICE,
            "",
            ZH_CHOICE_ANS,
            JA_CHOICE_ANS,
        ]),
        choice_row(),
    ]);
    let s = &pw.sheets[0];
    assert_eq!((s.total, s.usable), (3, 1));
    let reasons: Vec<(i64, &str)> = s
        .skipped
        .iter()
        .map(|r| (r.row, r.reason.as_str()))
        .collect();
    assert_eq!(reasons, [(1, SKIP6_NO_ENGLISH), (2, SKIP6_NO_ANSWER)]);
}

#[test]
fn header_rows_and_misaligned_translations() {
    let zh_short = "【中文翻译】\n一位园丁浇水。\nA. 薄荷\nB. 罗勒";
    let pw = parse(&[
        // A header row: ignored silently, not counted
        six([
            "Question (English)",
            "题目（中文）",
            "問題（日本語）",
            "Answer",
            "答案",
            "正解",
        ]),
        six([
            EN_CHOICE,
            zh_short,
            JA_CHOICE,
            EN_CHOICE_ANS,
            ZH_CHOICE_ANS,
            JA_CHOICE_ANS,
        ]),
    ]);
    let s = &pw.sheets[0];
    assert_eq!((s.total, s.usable), (1, 1));
    let q = only(&pw);
    assert_eq!(q.row, 2);
    // Two options instead of four: Chinese does not line up and is greyed out; its explanation is still kept
    assert!(q.zh.is_none());
    assert!(q.ja.is_some());
    assert!(q.explain.as_ref().and_then(|e| e.zh.as_ref()).is_some());
    assert!(s.warnings.is_empty());
}

#[test]
fn english_headings_and_answer_lines_without_headings() {
    // Mnemonic as heading, and an "Answer: …" line instead of 【Answer】
    let d = "Answer: D\n\n【Explanation】Parsley is the odd one out.\n\n【Mnemonic】P for parsley.";
    let en = "Which herb is not from the made-up list?\n\nA. Mint\n\nB. Basil\n\nC. Rosemary\n\nD. Parsley";
    let pw = parse(&[six([en, "", JA_CHOICE, d, "【答案】D", "【正解】D"])]);
    let q = only(&pw);
    assert_eq!(q.answer, Answer::Letters(strings(&["D"])));
    assert_eq!(q.kind, Kind::Single);
    assert!(q.zh.is_none());
    let ex = q.explain.as_ref().expect("explain");
    assert_eq!(
        ex.en.as_ref().map(|t| t.mnemonic.as_str()),
        Some("P for parsley.")
    );
    // E and F carry only an answer: those languages have no explanation entry
    assert!(ex.zh.is_none() && ex.ja.is_none());
    // Compatibility fields fall back to English
    assert_eq!(q.explanation, "Parsley is the odd one out.");
    assert!(pw.sheets[0].warnings.is_empty());
}

#[test]
fn four_column_files_keep_their_layout_and_six_column_ids_are_english() {
    // The same made-up question as a four-column row (A English, B Chinese, C Japanese, D answer)
    let four = parse(&[vec![
        Some(EN_CHOICE),
        Some(ZH_CHOICE),
        Some(JA_CHOICE),
        Some("【答案】B、C\n\n【解析·怎么选】说明。"),
    ]]);
    let q4 = only(&four);
    assert!(q4.lang.is_empty() && q4.ja.is_none() && q4.explain.is_none());
    assert!(q4.en.is_some());
    assert_eq!(q4.id, make_id("S", &q4.stem, "ミント"));
    let v4 = serde_json::to_value(q4).expect("json");
    for key in ["lang", "ja", "explain"] {
        assert!(
            v4.get(key).is_none(),
            "{key} must not appear for the four-column layout"
        );
    }
    // As a six-column row the identity is English: updating a four-column bank with a six-column file adds new questions
    let six_pw = parse(&[choice_row()]);
    assert_ne!(only(&six_pw).id, q4.id);
}

#[test]
fn warnings_reach_the_import_preview_and_result() {
    let zh_wrong = "【答案】A";
    let pw = parse(&[
        six([
            EN_CHOICE,
            ZH_CHOICE,
            JA_CHOICE,
            EN_CHOICE_ANS,
            zh_wrong,
            JA_CHOICE_ANS,
        ]),
        six(["", ZH_CHOICE, JA_CHOICE, EN_CHOICE_ANS, "", ""]),
    ]);
    let dir = tempfile::tempdir().expect("tmp");
    let store = Store::open(&dir.path().join("xexam.db")).expect("open");
    let plan = store.plan_import(&pw.sheets, &pw.questions).expect("plan");
    assert_eq!(plan["warnings"], 1);
    assert_eq!(plan["skipped"], 1);
    let preview = preview_payload(plan, "six.xlsx", "tok");
    let sheet = &preview["sheets"][0];
    assert_eq!(sheet["warnings"][0]["row"], 1);
    assert_eq!(sheet["warnings"][0]["reason"], WARN6_ZH_DIFFERS);
    assert_eq!(
        (
            sheet["warningsCount"].as_u64(),
            sheet["warningsMore"].as_u64()
        ),
        (Some(1), Some(0))
    );
    assert_eq!(sheet["skipped"][0]["reason"], SKIP6_NO_ENGLISH);

    // The stored question keeps the six-column keys (GET /api/bank)
    let result = store
        .apply_import("six.xlsx", &pw.sheets, &pw.questions)
        .expect("import");
    assert_eq!(result["added"], 1);
    let bank = store.bank().expect("bank");
    let q: &Value = &bank["questions"][0];
    assert_eq!(q["lang"], "en");
    assert_eq!(
        q["ja"]["stem"].as_str().map(|s| s.starts_with("庭師が")),
        Some(true)
    );
    assert_eq!(q["explain"]["en"]["note"], "confidence: high");
    // Column E has only an answer (no explanation, memory aid or note): no "zh" entry
    assert!(q["explain"].get("zh").is_none());

    // Re-importing the same file changes nothing; a changed explanation counts as updated
    let again = store.plan_import(&pw.sheets, &pw.questions).expect("plan");
    assert_eq!(
        (again["unchanged"].as_i64(), again["updated"].as_i64()),
        (Some(1), Some(0))
    );
    let changed_ja = JA_CHOICE_ANS.replace("植物です", "ハーブです");
    let pw2 = parse(&[six([
        EN_CHOICE,
        ZH_CHOICE,
        JA_CHOICE,
        EN_CHOICE_ANS,
        zh_wrong,
        &changed_ja,
    ])]);
    let plan2 = store
        .plan_import(&pw2.sheets, &pw2.questions)
        .expect("plan");
    assert_eq!(plan2["updated"], 1);
}

#[test]
fn english_hotspot_items_with_plain_spaces_keep_the_whole_sentence() {
    // The Yes / No after a single plain space (natural English input); Chinese and Japanese use a tab
    let en = "HOTSPOT\nFor each statement select Yes or No.\n1. Water is wet. Yes / No\n2. Fire is cold. Yes / No";
    let zh = "【中文翻译】\n热点题：对每条陈述选择是或否。\n1. 水是湿的。\t是 / 否\n2. 火是冷的。\t是 / 否";
    let ja = "ホットスポット\n各記述について、はいかいいえを選択してください。\n1. 水は濡れている。\tはい / いいえ\n2. 火は冷たい。\tはい / いいえ";
    let pw = parse(&[six([
        en,
        zh,
        ja,
        "【Answer】1 Yes; 2 No",
        "【答案】1 是；2 否",
        "【正解】1 はい、2 いいえ",
    ])]);
    let q = only(&pw);
    assert_eq!(q.kind, Kind::Hotspot);
    assert_eq!(
        q.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
        ["Water is wet.", "Fire is cold."]
    );
    assert_eq!(q.items[0].choices, strings(&["Yes", "No"]));
    assert_eq!(q.items[1].choices, strings(&["Yes", "No"]));
    assert_eq!(q.answer, Answer::Indexes(vec![0, 1]));
    assert!(
        pw.sheets[0].warnings.is_empty(),
        "{:?}",
        pw.sheets[0].warnings
    );
    assert!(
        q.zh.is_some() && q.ja.is_some(),
        "both translations line up"
    );
    // Multi-word choices still split at a tab or an ideographic space (the category fixture above), and a tab still wins
    // over plain spaces on the same line
    let en_tab = "HOTSPOT\nPick one.\n1. Mix the flour.\tFirst step / Last step\n2. Serve the cake.\tFirst step / Last step";
    let pw = parse(&[six([
        en_tab,
        "【中文翻译】
选一个。
1. 混合面粉。	第一步 / 最后一步
2. 端上蛋糕。	第一步 / 最后一步",
        "各項目を選んでください。
1. 小麦粉を混ぜる。	最初 / 最後
2. ケーキを出す。	最初 / 最後",
        "【Answer】1 First step; 2 Last step",
        "【答案】1 第一步；2 最后一步",
        "【正解】1 最初、2 最後",
    ])]);
    let q = only(&pw);
    assert_eq!(q.items[0].text, "Mix the flour.");
    assert_eq!(q.items[0].choices, strings(&["First step", "Last step"]));
    assert_eq!(q.answer, Answer::Indexes(vec![0, 1]));
    assert!(
        pw.sheets[0].warnings.is_empty(),
        "{:?}",
        pw.sheets[0].warnings
    );
}

#[test]
fn an_english_hotspot_marker_with_a_colon_is_removed_from_the_stem() {
    let en = "HOTSPOT: For each statement about a square, select Yes or No.\n1. A square has four sides.\tYes / No\n2. A square has three corners.\tYes / No";
    let pw = parse(&[six([
        en,
        ZH_HOT_YN,
        JA_HOT_YN,
        EN_HOT_YN_ANS,
        ZH_HOT_YN_ANS,
        JA_HOT_YN_ANS,
    ])]);
    let q = only(&pw);
    assert_eq!(q.kind, Kind::Hotspot);
    assert_eq!(
        q.stem,
        "For each statement about a square, select Yes or No."
    );
    assert_eq!(q.items.len(), 2);
    assert_eq!(q.answer, Answer::Indexes(vec![0, 1]));
    // the id is made from the stem without the marker
    let base = |text: &str| {
        let pw = parse(&[six([
            text,
            ZH_HOT_YN,
            JA_HOT_YN,
            EN_HOT_YN_ANS,
            ZH_HOT_YN_ANS,
            JA_HOT_YN_ANS,
        ])]);
        only(&pw).id.clone()
    };
    assert_eq!(
        base(en),
        base("HOTSPOT\nFor each statement about a square, select Yes or No.\n1. A square has four sides.\tYes / No\n2. A square has three corners.\tYes / No")
    );
}
