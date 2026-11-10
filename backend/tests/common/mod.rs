//! Shared helpers for the database integration tests.
#![allow(dead_code)]
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use xexam_backend::database::{FixedClock, Store};
use xexam_backend::models::{Answer, Item, Kind, Opt, ParsedSheet, Question, Zh};

/// 仓库根目录 = backend/ 的上一级（不写死任何机器上的路径）。
pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}
pub const FIXED_NOW: &str = "2026-03-04 05:06";

pub fn baseline(rel: &str) -> PathBuf {
    repo().join("tests/baseline").join(rel)
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
        english: String::new(),
        en: None,
        zh: Some(Zh {
            stem: "中文题干".into(),
            options: Some(vec!["是".into(), "否".into()]),
            items: None,
            choices: None,
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
        choices: None,
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

pub fn py_dumps(v: &Value) -> String {
    xexam_backend::database::pyjson::to_string(v).expect("dumps")
}
