//! Shared helpers for the database integration tests.
#![allow(dead_code)]
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use xexam_backend::database::{FixedClock, Store};
use xexam_backend::models::{Answer, Item, Kind, Opt, ParsedSheet, Question, Zh};

pub const REPO: &str = "C:/Users/Min/Desktop/Claude/xexam";
pub const FIXED_NOW: &str = "2026-03-04 05:06";

pub fn baseline(rel: &str) -> PathBuf {
    Path::new(REPO).join("tests/baseline").join(rel)
}

pub fn oracle_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/common/py_oracle.py")
}

/// True when `python` is on PATH.
pub fn python_available() -> bool {
    Command::new("python")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run the Python oracle. `None` (with a message on stderr) when python is missing.
pub fn py(args: &[&str], stdin: Option<&str>) -> Option<Value> {
    if !python_available() {
        eprintln!("SKIPPED: `python` is not on PATH, cannot run the Python oracle");
        return None;
    }
    let script = oracle_script();
    let mut cmd = Command::new("python");
    cmd.arg("-I").arg("-B").arg(&script).arg(REPO).args(args);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn python");
    if let Some(mut si) = child.stdin.take() {
        if let Some(s) = stdin {
            si.write_all(s.as_bytes()).expect("write stdin");
        }
    }
    let o = child.wait_with_output().expect("python output");
    assert!(
        o.status.success(),
        "python oracle failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    Some(serde_json::from_slice(&o.stdout).expect("oracle json"))
}

pub fn fixed(now: &str) -> Arc<FixedClock> {
    Arc::new(FixedClock(now.to_string()))
}

pub fn open_fixed(path: &Path) -> Store {
    Store::open_with_clock(path, fixed(FIXED_NOW)).expect("open store")
}

pub fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// Build a parsed single-choice question by hand.
pub fn single(sheet: &str, id: &str, stem: &str, row: i64) -> Question {
    Question {
        kind: Kind::Single,
        tag: String::new(),
        stem: stem.to_string(),
        options: vec![
            Opt {
                key: "A".into(),
                text: "はい".into(),
            },
            Opt {
                key: "B".into(),
                text: "いいえ".into(),
            },
        ],
        items: vec![],
        answer: Answer::Letters(vec!["B".into()]),
        translation: "中文翻译\nA. 是\nB. 否".into(),
        zh: Some(Zh {
            stem: "中文题干".into(),
            options: Some(vec!["是".into(), "否".into()]),
            items: None,
        }),
        explanation: "解析 \"quoted\" \\ back".into(),
        mnemonic: "口诀".into(),
        note: String::new(),
        id: id.to_string(),
        sheet: sheet.to_string(),
        row,
    }
}

pub fn multi(sheet: &str, id: &str, stem: &str, row: i64) -> Question {
    let mut q = single(sheet, id, stem, row);
    q.kind = Kind::Multi;
    q.answer = Answer::Letters(vec!["A".into(), "B".into()]);
    q
}

pub fn hotspot(sheet: &str, id: &str, stem: &str, row: i64) -> Question {
    let mut q = single(sheet, id, stem, row);
    q.kind = Kind::Hotspot;
    q.tag = "ホットスポット".into();
    q.options = vec![];
    q.items = vec![Item {
        text: "太陽は東から昇る。".into(),
        choices: vec!["はい".into(), "いいえ".into()],
    }];
    q.answer = Answer::Indexes(vec![0]);
    q.zh = Some(Zh {
        stem: "对每句话选择".into(),
        options: None,
        items: Some(vec!["太阳从东边升起。".into()]),
    });
    q
}

pub fn sheet(name: &str, usable: i64) -> ParsedSheet {
    ParsedSheet {
        name: name.to_string(),
        total: usable,
        usable,
        skipped: vec![],
    }
}

/// The same question as the Python-side dict (what `parse_workbook` would produce).
pub fn q_json(q: &Question) -> Value {
    serde_json::to_value(q).expect("question json")
}

pub fn sheets_json(s: &[ParsedSheet]) -> Value {
    serde_json::to_value(s).expect("sheets json")
}

/// A scenario file for `py_oracle.py build`.
pub fn scenario(imports: &[(&str, &[ParsedSheet], &[Question])], attempts: Value) -> String {
    let imps: Vec<Value> = imports
        .iter()
        .map(|(f, s, q)| json!({"filename": f, "sheets": sheets_json(s), "questions": q.iter().map(q_json).collect::<Vec<_>>()}))
        .collect();
    json!({"imports": imps, "attempts": attempts}).to_string()
}

pub fn py_dumps(v: &Value) -> String {
    xexam_backend::database::pyjson::to_string(v).expect("dumps")
}
