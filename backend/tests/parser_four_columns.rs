//! 四列格式（A 英语、B 中文、C 日语、D 答案·解析·口诀）和旧的三列格式（A 日语、B 中文、C 答案…）的并存。
//! 在内存里造 xlsx，不依赖任何真实题库。
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

/// 每行是若干列的文字（None = 这一格是空的），列从 A 开始。
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
    // 同一道题，三列格式和四列格式必须得到同一个 id（学习记录按 id 存）
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
    // 其余字段（不含英语）完全相同
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
    // 三列格式：C 列放答案，D 列没有东西 -> 保持原来的行为
    let data = workbook(&[vec![Some(JA_CHOICE), Some(ZH_CHOICE), Some(ANS_B)]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert!(q.stem.starts_with("これは十分に長い日本語"));
    assert_eq!(q.answer, Answer::Letters(vec!["B".to_string()]));
    // 三列格式但 D 列里被人写了备注（不含答案标题）-> 仍是三列
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
    // 英语只有 2 个选项、日语有 3 个 -> 不能一一对应，不提供英语切换，但原文保留
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
    // 热点题：英语小项个数不同
    let one_item = "HOTSPOT\nSelect.\n\n1. The sun rises in the east.\tYes / No";
    let data = workbook(&[vec![
        Some(one_item),
        Some(ZH_HOT),
        Some(JA_HOT),
        Some(ANS_HOT),
    ]]);
    assert!(only(&parse_workbook(&data).expect("parse")).en.is_none());
    // 热点题：英文选项个数和日语不同（3 个 vs 2 个）。英语的文字仍然可用，但选项不采用（位置就是答案下标，对不上会选错），
    // 界面这时给英语显示日语的选项。
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
    // 先放两行四列，再放一行三列（人为拼起来的）：多数决，不 panic
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
    // 前两行读成题目；第三行按四列读时 C 列是答案（没有选项结构）-> 读不出来，被登记为 skipped 而不是静默丢掉
    assert_eq!(pw.questions.len(), 2);
    let s = pw.sheets.first().expect("sheet");
    assert_eq!(s.total, 3);
    assert_eq!(s.skipped.len(), 1);
}

// ---------------------------------------------------------------- 热点题的选项：每种语言用自己那一列里写的选项

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
        // 日语仍是答案原文；答案下标来自日语
        assert_eq!(
            q.items[0].choices,
            vec!["はい".to_string(), "いいえ".to_string()]
        );
        assert_eq!(q.answer, Answer::Indexes(vec![0, 1]));
        // 中文：小项文字里不再带着「是 / 否」，选项单独成列
        let z = q.zh.as_ref().expect("zh");
        assert_eq!(
            z.items.as_deref(),
            Some(&["太阳从东边升起。".to_string(), "月亮是恒星。".to_string()][..])
        );
        assert_eq!(
            choices_of(&q.zh),
            Some(vec!["是|否".to_string(), "是|否".to_string()])
        );
        // 英语
        assert_eq!(
            choices_of(&q.en),
            Some(vec!["Yes|No".to_string(), "Yes|No".to_string()])
        );
    }
}

#[test]
fn a_language_without_trailing_choices_has_no_choices_and_the_ui_falls_back_to_japanese() {
    // 旧写法的中文（小项末尾没有选项）：照样能读，choices 为空
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
    // 中文写了 3 个选项、日语只有 2 个：不采用（位置就是答案下标，对不上就会选错）
    let zh = "【中文翻译】\n热点题：请选择。\n1. 太阳从东边升起。\t是 / 否 / 不确定\n2. 月亮是恒星。\t是 / 否 / 不确定";
    let data = workbook(&[vec![Some(EN_HOT), Some(zh), Some(JA_HOT), Some(ANS_HOT)]]);
    let pw = parse_workbook(&data).expect("parse");
    let q = only(&pw);
    assert!(q.zh.is_some(), "the Chinese text itself is still usable");
    assert_eq!(choices_of(&q.zh), None);
    // 只有第一个小项写了选项：也不采用
    let zh = "【中文翻译】\n热点题：请选择。\n1. 太阳从东边升起。\t是 / 否\n2. 月亮是恒星。";
    let data = workbook(&[vec![Some(EN_HOT), Some(zh), Some(JA_HOT), Some(ANS_HOT)]]);
    assert_eq!(
        choices_of(&only(&parse_workbook(&data).expect("parse")).zh),
        None
    );
}

#[test]
fn the_answer_is_still_read_from_the_japanese_choices_whatever_language_the_other_columns_use() {
    // 答案写成日语原文 / 写成序号以外的写法都不受其它列影响；中文列里写的「是 / 否」不会被当成答案的候选
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

// ---------------------------------------------------------------- 热点题的答案可以用三种语言里任意一种的选项词来写

const JA_LAYER: &str = "ホットスポット\n分類してください。\n\n1. 請求書の種類を分類する。\t前処理 / モデルステージ / 後処理\n\n2. 検証済みレコードを保存する。\t前処理 / モデルステージ / 後処理";
const ZH_LAYER: &str = "【中文翻译】\n热点题：请分类。\n1. 对发票类型进行分类。\t前处理 / 模型阶段 / 后处理\n2. 保存验证过的记录。\t前处理 / 模型阶段 / 后处理";
const EN_LAYER: &str = "HOTSPOT\nPlease classify.\n\n1. Classify the invoice type.\tPre-Processing / Model Stage / Post-Processing\n\n2. Save the validated records.\tPre-Processing / Model Stage / Post-Processing";

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
    // 同一个答案（第 1 项选第 2 个、第 2 项选第 3 个），三种写法必须得到同样的下标
    for ans in [
        "【答案】1 モデルステージ；2 後処理",
        "【答案】1 模型阶段；2 后处理",
        "【答案】1 Model Stage；2 Post-Processing",
        "【答案】1 モデルステージ；2 Post-Processing", // 混着写也行
    ] {
        let pw = parse_workbook(&workbook(&layer_answer(ans))).expect("parse");
        let q = only(&pw);
        assert_eq!(q.answer, Answer::Indexes(vec![1, 2]), "answer text: {ans}");
        assert_eq!(
            choices_of(&q.zh),
            Some(vec!["前处理|模型阶段|后处理".to_string(); 2])
        );
        assert_eq!(
            choices_of(&q.en),
            Some(vec![
                "Pre-Processing|Model Stage|Post-Processing".to_string();
                2
            ])
        );
    }
}

#[test]
fn a_word_that_is_in_none_of_the_languages_is_still_rejected_with_the_old_message() {
    let pw = parse_workbook(&workbook(&layer_answer("【答案】1 Maybe；2 後処理"))).expect("parse");
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
    // 英语那一列写了 2 个选项（日语是 3 个）：位置对不上，所以英文词不能用来写答案（宁可读不出来，也不选错）
    let en = "HOTSPOT\nPlease classify.\n\n1. Classify the invoice type.\tPre-Processing / Post-Processing\n\n2. Save the validated records.\tPre-Processing / Post-Processing";
    let rows = vec![vec![
        Some(en),
        Some(ZH_LAYER),
        Some(JA_LAYER),
        Some("【答案】1 Post-Processing；2 後処理"),
    ]];
    let pw = parse_workbook(&workbook(&rows)).expect("parse");
    assert_eq!(
        pw.questions.len(),
        0,
        "an English word from a misaligned list must not be accepted"
    );
    // 同样的答案，用日语词写就没问题
    let rows = vec![vec![
        Some(en),
        Some(ZH_LAYER),
        Some(JA_LAYER),
        Some("【答案】1 後処理；2 後処理"),
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
