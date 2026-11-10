//! Coexistence of the four-column format (A English, B Chinese, C Japanese, D answer/explanation/mnemonic) and the old three-column format (A Japanese, B Chinese, C answer...).
//! Builds the xlsx in memory; does not depend on any real question bank.
use std::io::{Cursor, Write};
use xexam_backend::models::{Answer, Kind, Question};
use xexam_backend::parser::{check_xlsx, parse_workbook};

const WB: &str = r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
const RELS: &str = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Target="worksheets/a.xml"/></Relationships>"#;

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
    zip_of(&[
        ("xl/workbook.xml", WB.as_bytes()),
        ("xl/_rels/workbook.xml.rels", RELS.as_bytes()),
        ("xl/worksheets/a.xml", xml.as_bytes()),
    ])
}

const JA_CHOICE: &str = "これは十分に長い日本語の問題文です。どれが正しいですか。\n\nA. 甲です\n\nB. 乙です\n\nC. 丙です";
const EN_CHOICE: &str = "This is a sufficiently long English question. Which is correct?\n\nA. Alpha\n\nB. Beta\n\nC. Gamma";
const ZH_CHOICE: &str = "【中文翻译】\n这是一个足够长的中文题目。哪个正确？\nA. 甲\nB. 乙\nC. 丙";
const ANS_B: &str = "【答案】B\n\n【解析·怎么选】因为乙。\n\n【记忆口诀】选乙。";

const JA_HOT: &str = "ホットスポット\n次の記述について選択してください。\n\n1. 太陽は東から昇る。\tはい / いいえ\n\n2. 月は恒星である。\tはい / いいえ";
const EN_HOT: &str = "HOTSPOT\nSelect for each of the following statements.\n\n1. The sun rises in the east.\tYes / No\n\n2. The moon is a star.\tYes / No";
const ZH_HOT: &str =
    "【中文翻译】\n热点题：请对下列陈述作选择。\n1. 太阳从东边升起。\n2. 月亮是恒星。";
const ANS_HOT: &str = "【答案】1 はい；2 いいえ\n\n【解析·怎么选】太阳东升；月亮不是恒星。";

fn only(pw: &xexam_backend::models::ParsedWorkbook) -> &Question {
    assert_eq!(pw.questions.len(), 1, "skipped: {:?}", pw.sheets);
    &pw.questions[0]
}

#[test]
fn four_column_choice_question_gets_english_and_keeps_japanese_as_the_stem() {
    let data = workbook(&[vec![
        Some(EN_CHOICE),
        Some(ZH_CHOICE),
        Some(JA_CHOICE),
        Some(ANS_B),
    ]]);
    check_xlsx(&data).expect("check");
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert_eq!(q.kind, Kind::Single);
    assert!(
        q.stem.starts_with("これは十分に長い日本語"),
        "stem must be the Japanese: {}",
        q.stem
    );
    assert_eq!(q.options.len(), 3);
    assert_eq!(q.options[0].text, "甲です");
    assert_eq!(q.answer, Answer::Letters(vec!["B".to_string()]));
    assert!(q.explanation.contains("因为乙"));
    assert!(q.mnemonic.contains("选乙"));
    let en = q.en.as_ref().expect("english structure");
    assert!(en
        .stem
        .starts_with("This is a sufficiently long English question"));
    assert_eq!(
        en.options.as_deref(),
        Some(&["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()][..])
    );
    assert!(q.english.starts_with("This is a sufficiently long"));
    let zh = q.zh.as_ref().expect("chinese structure");
    assert_eq!(zh.options.as_ref().map(Vec::len), Some(3));
}

#[test]
fn four_column_hotspot_question_gets_english_items_and_choices() {
    let data = workbook(&[vec![
        Some(EN_HOT),
        Some(ZH_HOT),
        Some(JA_HOT),
        Some(ANS_HOT),
    ]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert_eq!(q.kind, Kind::Hotspot);
    assert_eq!(q.items.len(), 2);
    assert_eq!(
        q.items[0].choices,
        vec!["はい".to_string(), "いいえ".to_string()]
    );
    assert_eq!(q.answer, Answer::Indexes(vec![0, 1]));
    let en = q.en.as_ref().expect("english structure");
    assert_eq!(en.stem, "Select for each of the following statements.");
    assert_eq!(
        en.items.as_deref(),
        Some(
            &[
                "The sun rises in the east.".to_string(),
                "The moon is a star.".to_string()
            ][..]
        )
    );
    assert_eq!(
        en.choices
            .as_ref()
            .map(|c| c.iter().map(|x| x.join("|")).collect::<Vec<_>>()),
        Some(vec!["Yes|No".to_string(), "Yes|No".to_string()])
    );
}

#[test]
fn the_question_id_comes_from_the_japanese_so_it_does_not_depend_on_the_layout() {
    // The same question must get the same id in the three-column and four-column formats (study records are stored by id)
    let three = workbook(&[vec![Some(JA_CHOICE), Some(ZH_CHOICE), Some(ANS_B)]]);
    let four = workbook(&[vec![
        Some(EN_CHOICE),
        Some(ZH_CHOICE),
        Some(JA_CHOICE),
        Some(ANS_B),
    ]]);
    let (a, b) = (
        parse_workbook(&three).expect("3"),
        parse_workbook(&four).expect("4"),
    );
    assert_eq!(only(&a).id, only(&b).id);
    assert!(
        only(&a).en.is_none() && only(&a).english.is_empty(),
        "three-column has no english"
    );
    // All other fields (excluding English) are identical
    let (qa, qb) = (only(&a), only(&b));
    assert_eq!(
        (
            &qa.stem,
            &qa.options,
            &qa.answer,
            &qa.explanation,
            &qa.mnemonic
        ),
        (
            &qb.stem,
            &qb.options,
            &qb.answer,
            &qb.explanation,
            &qb.mnemonic
        )
    );
}

#[test]
fn an_old_three_column_sheet_is_never_mistaken_for_four_columns() {
    // Three-column format: column C holds the answer, column D has nothing -> behavior unchanged
    let data = workbook(&[vec![Some(JA_CHOICE), Some(ZH_CHOICE), Some(ANS_B)]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert!(q.stem.starts_with("これは十分に長い日本語"));
    assert_eq!(q.answer, Answer::Letters(vec!["B".to_string()]));
    // Three-column format, but someone wrote a note in column D (no answer heading) -> still three-column
    let with_note = workbook(&[vec![
        Some(JA_CHOICE),
        Some(ZH_CHOICE),
        Some(ANS_B),
        Some("随手写的备注"),
    ]]);
    let pw = parse_workbook(&with_note).expect("parse");
    assert!(only(&pw).stem.starts_with("これは十分に長い日本語"));
}

#[test]
fn a_four_column_question_without_english_still_works_and_has_no_en() {
    let data = workbook(&[vec![None, Some(ZH_CHOICE), Some(JA_CHOICE), Some(ANS_B)]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert!(q.en.is_none());
    assert!(q.english.is_empty());
    assert!(q.stem.starts_with("これは十分に長い日本語"));
}

#[test]
fn english_that_does_not_line_up_with_the_japanese_gives_no_en_but_is_kept() {
    // English has only 2 options while Japanese has 3 -> no one-to-one mapping, so no English switch is offered, but the original text is kept
    let short_en = "A question in English that is long enough.\n\nA. Alpha\n\nB. Beta";
    let data = workbook(&[vec![
        Some(short_en),
        Some(ZH_CHOICE),
        Some(JA_CHOICE),
        Some(ANS_B),
    ]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert!(
        q.en.is_none(),
        "mismatched option count must not produce en"
    );
    assert_eq!(q.english, short_en);
    // Hotspot question: the number of English sub-items differs
    let one_item = "HOTSPOT\nSelect.\n\n1. The sun rises in the east.\tYes / No";
    let data = workbook(&[vec![
        Some(one_item),
        Some(ZH_HOT),
        Some(JA_HOT),
        Some(ANS_HOT),
    ]]);
    assert!(only(&parse_workbook(&data).expect("parse")).en.is_none());
    // Hotspot question: the number of English options differs from Japanese (3 vs 2). The English text is still usable, but the options are not adopted (position is the answer index; a mismatch would select the wrong one),
    // so the UI shows the Japanese options for English.
    let wrong_choices = "HOTSPOT\nSelect.\n\n1. The sun rises in the east.\tYes / No / Maybe\n\n2. The moon is a star.\tYes / No / Maybe";
    let data = workbook(&[vec![
        Some(wrong_choices),
        Some(ZH_HOT),
        Some(JA_HOT),
        Some(ANS_HOT),
    ]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    let en = q.en.as_ref().expect("the English text is still usable");
    assert_eq!(en.items.as_ref().map(Vec::len), Some(2));
    assert!(
        en.choices.is_none(),
        "mismatching choices must not be adopted"
    );
    assert_eq!(
        q.items[0].choices,
        vec!["はい".to_string(), "いいえ".to_string()]
    );
}

#[test]
fn a_sheet_with_both_layouts_of_rows_uses_the_majority_and_never_panics() {
    // Put two four-column rows first, then one three-column row (artificially stitched together): majority vote, no panic
    let data = workbook(&[
        vec![
            Some(EN_CHOICE),
            Some(ZH_CHOICE),
            Some(JA_CHOICE),
            Some(ANS_B),
        ],
        vec![
            Some(EN_CHOICE),
            Some(ZH_CHOICE),
            Some(JA_CHOICE.replace("甲です", "甲ですね").leak() as &str),
            Some(ANS_B),
        ],
        vec![Some(JA_CHOICE), Some(ZH_CHOICE), Some(ANS_B)],
    ]);
    let pw = parse_workbook(&data).expect("parse");
    // The first two rows are read as questions; read as four-column, the third row has an answer in column C (no option structure) -> cannot be read, so it is recorded as skipped rather than silently dropped
    assert_eq!(pw.questions.len(), 2);
    let s = pw.sheets.first().expect("sheet");
    assert_eq!(s.total, 3);
    assert_eq!(s.skipped.len(), 1);
}

// ---------------------------------------------------------------- Hotspot question options: each language uses the options written in its own column

const ZH_HOT_CH: &str = "【中文翻译】\n热点题：请对下列陈述作选择。\n1. 太阳从东边升起。\t是 / 否\n2. 月亮是恒星。\t是 / 否";
const ZH_HOT_CH_JOINED: &str = "【中文翻译】\n热点题：请对下列陈述作选择。\n1. 太阳从东边升起。　是 / 否\n2. 月亮是恒星。　是 / 否";

fn choices_of(v: &Option<xexam_backend::models::Zh>) -> Option<Vec<String>> {
    v.as_ref()
        .and_then(|z| z.choices.as_ref())
        .map(|c| c.iter().map(|x| x.join("|")).collect())
}

#[test]
fn each_language_gets_its_own_hotspot_choices_from_its_own_column() {
    for zh in [ZH_HOT_CH, ZH_HOT_CH_JOINED] {
        let data = workbook(&[vec![Some(EN_HOT), Some(zh), Some(JA_HOT), Some(ANS_HOT)]]);
        let pw = parse_workbook(&data).expect("parse");
        let q = only(&pw);
        // Japanese is still the original answer text; the answer indices come from Japanese
        assert_eq!(
            q.items[0].choices,
            vec!["はい".to_string(), "いいえ".to_string()]
        );
        assert_eq!(q.answer, Answer::Indexes(vec![0, 1]));
        // Chinese: the sub-item text no longer carries the literal "是 / 否"; the options form a separate column
        let z = q.zh.as_ref().expect("zh");
        assert_eq!(
            z.items.as_deref(),
            Some(&["太阳从东边升起。".to_string(), "月亮是恒星。".to_string()][..])
        );
        assert_eq!(
            choices_of(&q.zh),
            Some(vec!["是|否".to_string(), "是|否".to_string()])
        );
        // English
        assert_eq!(
            choices_of(&q.en),
            Some(vec!["Yes|No".to_string(), "Yes|No".to_string()])
        );
    }
}

#[test]
fn a_language_without_trailing_choices_has_no_choices_and_the_ui_falls_back_to_japanese() {
    // Old-style Chinese (no options at the end of the sub-items): still readable, choices are empty
    let data = workbook(&[vec![
        Some(EN_HOT),
        Some(ZH_HOT),
        Some(JA_HOT),
        Some(ANS_HOT),
    ]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert!(q.zh.is_some());
    assert_eq!(choices_of(&q.zh), None);
    assert_eq!(
        choices_of(&q.en),
        Some(vec!["Yes|No".to_string(), "Yes|No".to_string()])
    );
}

#[test]
fn choices_that_do_not_line_up_one_for_one_are_dropped_not_guessed() {
    // Chinese has 3 options, Japanese only 2: not adopted (position is the answer index; a mismatch would select the wrong one)
    let zh = "【中文翻译】\n热点题：请选择。\n1. 太阳从东边升起。\t是 / 否 / 不确定\n2. 月亮是恒星。\t是 / 否 / 不确定";
    let data = workbook(&[vec![Some(EN_HOT), Some(zh), Some(JA_HOT), Some(ANS_HOT)]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert!(q.zh.is_some(), "the Chinese text itself is still usable");
    assert_eq!(choices_of(&q.zh), None);
    // Only the first sub-item has options: not adopted either
    let zh = "【中文翻译】\n热点题：请选择。\n1. 太阳从东边升起。\t是 / 否\n2. 月亮是恒星。";
    let data = workbook(&[vec![Some(EN_HOT), Some(zh), Some(JA_HOT), Some(ANS_HOT)]]);
    assert_eq!(
        choices_of(&only(&parse_workbook(&data).expect("parse")).zh),
        None
    );
}

#[test]
fn the_answer_is_still_read_from_the_japanese_choices_whatever_language_the_other_columns_use() {
    // Answers written as the Japanese original text / in any form other than the ordinal are unaffected by the other columns; the "是 / 否" written in the Chinese column is not treated as an answer candidate
    let ans_zh_style = "【答案】1 是；2 否";
    let data = workbook(&[vec![
        Some(EN_HOT),
        Some(ZH_HOT_CH),
        Some(JA_HOT),
        Some(ans_zh_style),
    ]]);
    let pw = parse_workbook(&data).expect("parse");
    assert_eq!(
        only(&pw).answer,
        Answer::Indexes(vec![0, 1]),
        "是/否 are accepted as yes/no for the Japanese choices"
    );
}

// ---------------------------------------------------------------- A hotspot question answer can be written with the option words of any one of the three languages

const JA_LAYER: &str = "ホットスポット\nどの段階ですか。\n\n1. スープを 20 分間煮込む。\t下ごしらえ / 調理 / 盛り付け\n\n2. スープを器に注ぐ。\t下ごしらえ / 調理 / 盛り付け";
const ZH_LAYER: &str = "【中文翻译】\n热点题：属于哪个阶段？\n1. 把汤小火炖 20 分钟。\t准备 / 烹饪 / 上菜\n2. 把汤盛进碗里。\t准备 / 烹饪 / 上菜";
const EN_LAYER: &str = "HOTSPOT\nWhich stage is it?\n\n1. Simmer the soup for 20 minutes.\tPreparation / Cooking / Serving\n\n2. Pour the soup into bowls.\tPreparation / Cooking / Serving";

fn layer_answer(ans: &str) -> Vec<Vec<Option<&str>>> {
    vec![vec![
        Some(EN_LAYER),
        Some(ZH_LAYER),
        Some(JA_LAYER),
        Some(ans),
    ]]
}

#[test]
fn answers_may_be_written_with_the_choice_words_of_any_of_the_three_languages() {
    // The same answer (item 1 picks the 2nd option, item 2 picks the 3rd), written in the three notations, must yield the same indices
    for ans in [
        "【答案】1 調理；2 盛り付け",
        "【答案】1 烹饪；2 上菜",
        "【答案】1 Cooking；2 Serving",
        "【答案】1 調理；2 Serving", // mixing notations is fine too
    ] {
        let pw = parse_workbook(&workbook(&layer_answer(ans))).expect("parse");
        let q = only(&pw);
        assert_eq!(q.answer, Answer::Indexes(vec![1, 2]), "answer text: {ans}");
        assert_eq!(
            choices_of(&q.zh),
            Some(vec!["准备|烹饪|上菜".to_string(); 2])
        );
        assert_eq!(
            choices_of(&q.en),
            Some(vec!["Preparation|Cooking|Serving".to_string(); 2])
        );
    }
}

#[test]
fn a_word_that_is_in_none_of_the_languages_is_still_rejected_with_the_old_message() {
    let pw =
        parse_workbook(&workbook(&layer_answer("【答案】1 Maybe；2 盛り付け"))).expect("parse");
    assert_eq!(pw.questions.len(), 0);
    let skipped = &pw.sheets.first().expect("sheet").skipped;
    assert_eq!(skipped.len(), 1);
    assert!(
        skipped[0]
            .reason
            .contains("第 1 小项的答案「Maybe」不在选项"),
        "{}",
        skipped[0].reason
    );
}

#[test]
fn other_language_words_only_count_when_their_column_lines_up_one_for_one_with_the_japanese() {
    // The English column has 2 options (Japanese has 3): positions do not match, so English words cannot be used to write the answer (better to fail to read than to select the wrong option)
    let en = "HOTSPOT\nWhich stage is it?\n\n1. Simmer the soup for 20 minutes.\tPreparation / Serving\n\n2. Pour the soup into bowls.\tPreparation / Serving";
    let rows = vec![vec![
        Some(en),
        Some(ZH_LAYER),
        Some(JA_LAYER),
        Some("【答案】1 Serving；2 盛り付け"),
    ]];
    let pw = parse_workbook(&workbook(&rows)).expect("parse");
    assert_eq!(
        pw.questions.len(),
        0,
        "an English word from a misaligned list must not be accepted"
    );
    // The same answer written with Japanese words works fine
    let rows = vec![vec![
        Some(en),
        Some(ZH_LAYER),
        Some(JA_LAYER),
        Some("【答案】1 盛り付け；2 盛り付け"),
    ]];
    let pw = parse_workbook(&workbook(&rows)).expect("parse");
    assert_eq!(only(&pw).answer, Answer::Indexes(vec![2, 2]));
}

#[test]
fn yes_no_questions_accept_yes_no_in_all_three_languages() {
    for ans in [
        "【答案】1 はい；2 いいえ",
        "【答案】1 是；2 否",
        "【答案】1 Yes；2 No",
    ] {
        let data = workbook(&[vec![Some(EN_HOT), Some(ZH_HOT_CH), Some(JA_HOT), Some(ans)]]);
        assert_eq!(
            only(&parse_workbook(&data).expect("parse")).answer,
            Answer::Indexes(vec![0, 1]),
            "{ans}"
        );
    }
}
