//! Compares against the output of the old (Python) parser: the demo + 49 fixtures.
//! Fixtures and golden files are under tests/baseline in the repository (read-only).

use serde_json::Value;
use std::path::PathBuf;
use xexam_backend::parser::{check_xlsx, parse_workbook};

fn baseline() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("tests/baseline")
}

/// Fixtures that intentionally differ from the old version (recorded in docs/migration-plan.md Q7 and docs/audit/xlsx-parser.md §8), with the prefix of their new result.
const INTENTIONAL: &[(&str, &str)] = &[
    // DOCTYPE + entity in UTF-16 / non-.xml suffix / uppercase suffix: the old version let it through and expanded it; the new version always rejects it
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
    // The zip has two entries with the same name: the old version takes the last one, the new version rejects it (the check and the read might see different entries)
    (
        "f20j_duplicate_zip_entries",
        "这个 .xlsx 文件已损坏，打不开。",
    ),
];

/// The old error text embeds library text: compare only the Chinese prefix.
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
    // Sheets in UTF-8 BOM and UTF-16 files are both readable in the old version
    for n in ["f22a_utf8_bom_sheet", "f22b_utf16_sheet"] {
        let data =
            std::fs::read(baseline().join("fixtures").join(format!("{n}.xlsx"))).expect("fixture");
        assert_eq!(run(&data).expect(n).questions.len(), 1, "{n}");
    }
}
