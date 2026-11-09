//! 对照旧版（Python）解析器的输出：demo + 49 个夹具。
//! 夹具与 golden 在仓库的 tests/baseline 下（只读）。

use serde_json::Value;
use std::path::PathBuf;
use xexam_backend::parser::{check_xlsx, parse_workbook};

fn baseline() -> PathBuf {
    PathBuf::from("C:/Users/Min/Desktop/Claude/xexam/tests/baseline")
}

/// 与旧版刻意不同（登记在 docs/migration-plan.md Q7 与 docs/audit/xlsx-parser.md §8）的夹具及其新结果前缀。
const INTENTIONAL: &[(&str, &str)] = &[
    // UTF-16 / 非 .xml 后缀 / 大写后缀里的 DOCTYPE + 实体：旧版放行并展开，新版一律拒绝
    (
        "f26a_doctype_in_utf16_sheet",
        "文件里含有不被允许的内容（XML 实体声明）",
    ),
    (
        "f26b_doctype_in_nonxml_extension",
        "文件里含有不被允许的内容（XML 实体声明）",
    ),
    (
        "f26c_doctype_in_uppercase_ext",
        "文件里含有不被允许的内容（XML 实体声明）",
    ),
    // zip 里有两个同名条目：旧版取最后一个，新版拒绝（检查与读取可能看到不同条目）
    (
        "f20j_duplicate_zip_entries",
        "这个 .xlsx 文件已损坏，打不开。",
    ),
];

/// 旧版错误文案里嵌了库的文字：只比较中文前缀。
fn prefix_of(msg: &str) -> &str {
    for p in ["文件里的 XML 读不了：", "这个文件缺少 .xlsx 必需的内容（"] {
        if msg.starts_with(p) {
            return p;
        }
    }
    msg
}

fn run(data: &[u8]) -> Result<xexam_backend::models::ParsedWorkbook, String> {
    check_xlsx(data).map_err(|e| e.to_string())?;
    parse_workbook(data).map_err(|e| e.to_string())
}

#[test]
fn every_fixture_matches_the_python_oracle() {
    let dir = baseline().join("parser_golden");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("golden dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json") && !n.starts_with('_'))
        .map(|n| n.trim_end_matches(".json").to_string())
        .collect();
    names.sort();
    assert!(names.len() >= 48, "found {} golden files", names.len());

    let mut intentional_seen = 0;
    let mut failures: Vec<String> = Vec::new();
    for name in &names {
        let golden: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join(format!("{name}.json"))).expect("read golden"),
        )
        .expect("golden json");
        let fixture = baseline().join("fixtures").join(format!("{name}.xlsx"));
        let data = std::fs::read(&fixture).unwrap_or_else(|e| panic!("{}: {e}", fixture.display()));
        let outcome = golden.get("server_outcome").expect("server_outcome");
        let got = run(&data);

        if let Some((_, want)) = INTENTIONAL.iter().find(|(n, _)| n == name) {
            intentional_seen += 1;
            match &got {
                Err(m) if m.starts_with(want) => {}
                other => failures.push(format!(
                    "{name}: expected intentional rejection {want:?}, got {:?}",
                    other.as_ref().map(|_| "ok")
                )),
            }
            continue;
        }

        let status = outcome.get("status").and_then(Value::as_i64).unwrap_or(-1);
        if status == 200 {
            let pw = match got {
                Ok(pw) => pw,
                Err(m) => {
                    failures.push(format!("{name}: expected success, got error {m}"));
                    continue;
                }
            };
            let want_q = outcome.get("questions").cloned().unwrap_or(Value::Null);
            let want_s = outcome.get("sheets").cloned().unwrap_or(Value::Null);
            let got_q = serde_json::to_value(&pw.questions).expect("ser");
            let got_s = serde_json::to_value(&pw.sheets).expect("ser");
            if got_s != want_s {
                failures.push(format!(
                    "{name}: sheets differ\n  want {want_s}\n  got  {got_s}"
                ));
            }
            match (got_q.as_array(), want_q.as_array()) {
                (Some(g), Some(w)) => {
                    if g.len() != w.len() {
                        failures.push(format!("{name}: {} questions, want {}", g.len(), w.len()));
                    }
                    for (i, (a, b)) in g.iter().zip(w).enumerate() {
                        if a != b {
                            failures.push(format!(
                                "{name}: question #{i} differs\n  want {b}\n  got  {a}"
                            ));
                            break;
                        }
                    }
                }
                _ => failures.push(format!("{name}: questions not arrays")),
            }
        } else {
            let want = outcome.get("error").and_then(Value::as_str).unwrap_or("");
            match got {
                Ok(_) => failures.push(format!("{name}: expected 400 {want:?}, got success")),
                Err(m) => {
                    if prefix_of(&m) != prefix_of(want) {
                        failures.push(format!("{name}: error differs\n  want {want}\n  got  {m}"));
                    }
                }
            }
        }
    }
    assert_eq!(intentional_seen, INTENTIONAL.len());
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn demo_workbook_has_five_questions() {
    let data = std::fs::read(baseline().join("fixtures").join("demo.xlsx")).expect("demo");
    let pw = run(&data).expect("demo parses");
    assert_eq!(pw.questions.len(), 5);
    assert!(pw.questions.iter().all(|q| q.zh.is_some()));
}

#[test]
fn text_encoding_cases_parse_like_python() {
    // UTF-8 BOM 与 UTF-16 的工作表在旧版里都能读
    for n in ["f22a_utf8_bom_sheet", "f22b_utf16_sheet"] {
        let data =
            std::fs::read(baseline().join("fixtures").join(format!("{n}.xlsx"))).expect("fixture");
        assert_eq!(run(&data).expect(n).questions.len(), 1, "{n}");
    }
}
