//! Drift guard for the database diagrams in `docs/{en,ja,zh-CN}/database.md` (AI database, v3.1).
//!
//! A fresh AI database is opened, every table is read with `PRAGMA table_info` and `PRAGMA foreign_key_list`, and the Mermaid
//! `erDiagram` blocks of the three database documents are compared with it:
//!
//! - section 8.2 (the full diagrams): every table is a "full" entity (at least one attribute line) exactly once, with its
//!   columns, declared types, `PK` and `FK` marks as stored; no full entity names a table that does not exist; stubs
//!   (entities without attributes, or names used only in a relation) name existing tables;
//! - section 9.2 (the overview, relations only): every declared foreign key is a solid relation between the two tables,
//!   every solid relation is a declared foreign key; dotted relations only need both names to be tables of this database
//!   or of the main database (the names of the main database are read from the entities of section 9.1 of the same document);
//! - the three languages agree on entities, columns, types and relations (comments may differ).
//!
//! The main database has the same test in `backend/tests/docs_diagrams.rs`. All content is made up.
mod common;

use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::store::{Store, DB_FILE};

const LANGS: [&str; 3] = ["en", "ja", "zh-CN"];

/// Section prefixes of the full diagrams and of the overview in this database's document.
const FULL_PREFIX: &str = "8.2";
const OVERVIEW_PREFIX: &str = "9.2";
/// Section of the other database's overview (its entity names are the legal targets of dotted relations here).
const OTHER_OVERVIEW_PREFIX: &str = "9.1";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Col {
    name: String,
    ty: String,
    marks: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct Entity {
    name: String,
    cols: Vec<Col>,
}

#[derive(Debug, Clone)]
struct Rel {
    a: String,
    b: String,
    solid: bool,
    line: String,
}

#[derive(Debug)]
struct Block {
    section: String,
    entities: Vec<Entity>,
    rels: Vec<Rel>,
}

/// The section number of a Markdown heading (`## 3. Tables` -> `3`, `### 8.2a Schema v5` -> `8.2a`), or None.
fn heading_number(line: &str) -> Option<String> {
    let rest = line.strip_prefix("##")?;
    let rest = rest.trim_start_matches('#').strip_prefix(' ')?;
    let token = rest.split_whitespace().next()?;
    let token = token.trim_end_matches('.');
    let first = token.chars().next()?;
    if !first.is_ascii_digit() {
        return None;
    }
    if token.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') {
        Some(token.to_string())
    } else {
        None
    }
}

/// Every Mermaid `erDiagram` block of a document with the number of the heading above it.
fn parse_doc(text: &str, label: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut section = String::new();
    let mut lines = text.lines().enumerate();
    while let Some((_, line)) = lines.next() {
        if let Some(n) = heading_number(line) {
            section = n;
            continue;
        }
        if line.trim_end() != "```mermaid" {
            continue;
        }
        let mut body: Vec<(usize, &str)> = Vec::new();
        for (i, l) in lines.by_ref() {
            if l.trim_end() == "```" {
                break;
            }
            body.push((i + 1, l));
        }
        let first = body.iter().map(|(_, l)| l.trim()).find(|l| !l.is_empty());
        if first != Some("erDiagram") {
            continue;
        }
        blocks.push(parse_block(&section, &body, label));
    }
    blocks
}

fn parse_block(section: &str, body: &[(usize, &str)], label: &str) -> Block {
    let mut entities: Vec<Entity> = Vec::new();
    let mut rels = Vec::new();
    let mut open: Option<Entity> = None;
    for (no, raw) in body {
        let line = raw.trim();
        if line.is_empty() || line == "erDiagram" {
            continue;
        }
        if let Some(e) = open.as_mut() {
            if line == "}" {
                entities.push(open.take().expect("open entity"));
                continue;
            }
            e.cols.push(parse_attr(line, label, section, &e.name, *no));
            continue;
        }
        if let Some(head) = line.strip_suffix('{') {
            let name = head.trim();
            assert!(
                !name.is_empty() && !name.contains(' '),
                "{label} section {section} line {no}: bad entity line `{line}`"
            );
            open = Some(Entity {
                name: name.to_string(),
                cols: Vec::new(),
            });
            continue;
        }
        if let Some(head) = line.strip_suffix("{ }").or_else(|| line.strip_suffix("{}")) {
            entities.push(Entity {
                name: head.trim().to_string(),
                cols: Vec::new(),
            });
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        assert!(
            parts.len() >= 3,
            "{label} section {section} line {no}: cannot read `{line}`"
        );
        let connector = parts[1];
        let solid = connector.contains("--");
        let dotted = connector.contains("..");
        assert!(
            solid != dotted,
            "{label} section {section} line {no}: `{connector}` is neither a solid nor a dotted relation"
        );
        rels.push(Rel {
            a: parts[0].to_string(),
            b: parts[2].to_string(),
            solid,
            line: line.to_string(),
        });
    }
    assert!(
        open.is_none(),
        "{label} section {section}: an entity block is not closed"
    );
    Block {
        section: section.to_string(),
        entities,
        rels,
    }
}

fn parse_attr(line: &str, label: &str, section: &str, entity: &str, no: usize) -> Col {
    // `TYPE name [PK,FK,UK] ["comment"]`; the comment may hold spaces, so cut it off first
    let head = match line.find('"') {
        Some(i) => line[..i].trim(),
        None => line,
    };
    let parts: Vec<&str> = head.split_whitespace().collect();
    assert!(
        parts.len() == 2 || parts.len() == 3,
        "{label} section {section} entity {entity} line {no}: cannot read the attribute `{line}`"
    );
    let marks: BTreeSet<String> = match parts.get(2) {
        Some(m) => m.split(',').map(str::to_string).collect(),
        None => BTreeSet::new(),
    };
    for m in &marks {
        assert!(
            m == "PK" || m == "FK" || m == "UK",
            "{label} section {section} entity {entity} line {no}: unknown mark `{m}`"
        );
    }
    Col {
        name: parts[1].to_string(),
        ty: parts[0].to_string(),
        marks,
    }
}

/// What the fresh database says: table -> columns (name, declared type, pk position), and the declared foreign keys.
struct Truth {
    tables: BTreeMap<String, Vec<(String, String, i64)>>,
    fk_columns: BTreeMap<String, BTreeSet<String>>,
    fk_pairs: BTreeSet<(String, String)>,
}

fn truth() -> Truth {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(DB_FILE);
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open a fresh store");
    drop(store);
    let c = Connection::open(&path).expect("open the file");
    let names: Vec<String> = {
        let mut st = c
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .expect("prepare");
        let rows = st.query_map([], |r| r.get::<_, String>(0)).expect("query");
        rows.collect::<Result<_, _>>().expect("rows")
    };
    let mut tables = BTreeMap::new();
    let mut fk_columns: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut fk_pairs = BTreeSet::new();
    for t in names {
        let mut st = c
            .prepare("SELECT name, type, pk FROM pragma_table_info(?1) ORDER BY cid")
            .expect("prepare table_info");
        let cols: Vec<(String, String, i64)> = st
            .query_map([&t], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("table_info")
            .collect::<Result<_, _>>()
            .expect("rows");
        let mut fk = c
            .prepare("SELECT \"from\", \"table\" FROM pragma_foreign_key_list(?1)")
            .expect("prepare foreign_key_list");
        let fks: Vec<(String, String)> = fk
            .query_map([&t], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("foreign_key_list")
            .collect::<Result<_, _>>()
            .expect("rows");
        for (from, to) in fks {
            fk_columns.entry(t.clone()).or_default().insert(from);
            fk_pairs.insert(pair(&t, &to));
        }
        tables.insert(t, cols);
    }
    Truth {
        tables,
        fk_columns,
        fk_pairs,
    }
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn in_section(section: &str, prefix: &str) -> bool {
    // `8.2` also covers `8.2a` .. `8.2k`; `9.2` covers `9.2.1`
    section == prefix
        || section.strip_prefix(prefix).is_some_and(|rest| {
            rest.starts_with('.') || rest.chars().all(|c| c.is_ascii_lowercase())
        })
}

fn docs() -> Vec<(&'static str, Vec<Block>)> {
    let root: PathBuf = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf();
    LANGS
        .iter()
        .map(|lang| {
            let path = root.join("docs").join(lang).join("database.md");
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            (*lang, parse_doc(&text, lang))
        })
        .collect()
}

#[test]
fn full_diagrams_match_the_fresh_database() {
    let truth = truth();
    for (lang, blocks) in docs() {
        let mut seen: BTreeMap<String, String> = BTreeMap::new();
        for b in blocks
            .iter()
            .filter(|b| in_section(&b.section, FULL_PREFIX))
        {
            for e in &b.entities {
                if !truth.tables.contains_key(&e.name) {
                    panic!(
                        "{lang} section {}: entity `{}` is not a table of the AI database",
                        b.section, e.name
                    );
                }
                if e.cols.is_empty() {
                    continue; // a stub
                }
                if let Some(first) = seen.insert(e.name.clone(), b.section.clone()) {
                    panic!(
                        "{lang} section {}: table `{}` is drawn in full twice (also in section {first})",
                        b.section, e.name
                    );
                }
                let want = &truth.tables[&e.name];
                let fks = truth.fk_columns.get(&e.name);
                if e.cols.len() != want.len() {
                    let have: Vec<&str> = e.cols.iter().map(|c| c.name.as_str()).collect();
                    let real: Vec<&str> = want.iter().map(|c| c.0.as_str()).collect();
                    panic!(
                        "{lang} section {} entity `{}`: {} columns drawn, the table has {}.\n  drawn: {have:?}\n  table: {real:?}",
                        b.section,
                        e.name,
                        e.cols.len(),
                        want.len()
                    );
                }
                for (i, (c, (name, ty, pk))) in e.cols.iter().zip(want).enumerate() {
                    if &c.name != name {
                        panic!(
                            "{lang} section {} entity `{}` column {}: drawn `{}`, the table has `{}`",
                            b.section,
                            e.name,
                            i + 1,
                            c.name,
                            name
                        );
                    }
                    if !c.ty.eq_ignore_ascii_case(ty) {
                        panic!(
                            "{lang} section {} entity `{}` column `{}`: drawn type `{}`, declared `{}`",
                            b.section, e.name, c.name, c.ty, ty
                        );
                    }
                    if c.marks.contains("PK") != (*pk > 0) {
                        panic!(
                            "{lang} section {} entity `{}` column `{}`: PK mark {} but the primary key {} it",
                            b.section,
                            e.name,
                            c.name,
                            if c.marks.contains("PK") { "drawn" } else { "missing" },
                            if *pk > 0 { "contains" } else { "does not contain" }
                        );
                    }
                    let is_fk = fks.is_some_and(|s| s.contains(name));
                    if c.marks.contains("FK") != is_fk {
                        panic!(
                            "{lang} section {} entity `{}` column `{}`: FK mark {} but the column {} a foreign key",
                            b.section,
                            e.name,
                            c.name,
                            if c.marks.contains("FK") { "drawn" } else { "missing" },
                            if is_fk { "is" } else { "is not" }
                        );
                    }
                }
            }
            for r in &b.rels {
                check_relation(lang, &b.section, r, &truth, &BTreeSet::new());
            }
        }
        for t in truth.tables.keys() {
            assert!(
                seen.contains_key(t),
                "{lang}: table `{t}` is not drawn in full in any diagram of section 3"
            );
        }
    }
}

fn check_relation(lang: &str, section: &str, r: &Rel, truth: &Truth, other: &BTreeSet<String>) {
    if r.solid {
        for n in [&r.a, &r.b] {
            assert!(
                truth.tables.contains_key(n),
                "{lang} section {section}: the solid relation `{}` names `{n}`, which is not a table of the AI database",
                r.line
            );
        }
        assert!(
            truth.fk_pairs.contains(&pair(&r.a, &r.b)),
            "{lang} section {section}: the solid relation `{}` is not a declared foreign key (draw it dotted if it is a soft link)",
            r.line
        );
    } else {
        for n in [&r.a, &r.b] {
            assert!(
                truth.tables.contains_key(n) || other.contains(n),
                "{lang} section {section}: the dotted relation `{}` names `{n}`, which is a table of neither database",
                r.line
            );
        }
    }
}

#[test]
fn overview_matches_the_declared_foreign_keys() {
    let truth = truth();
    for (lang, blocks) in docs() {
        let other: BTreeSet<String> = blocks
            .iter()
            .filter(|b| in_section(&b.section, OTHER_OVERVIEW_PREFIX))
            .flat_map(|b| {
                b.entities
                    .iter()
                    .map(|e| e.name.clone())
                    .chain(b.rels.iter().flat_map(|r| [r.a.clone(), r.b.clone()]))
            })
            .collect();
        assert!(
            !other.is_empty(),
            "{lang}: the overview of the main database (section {OTHER_OVERVIEW_PREFIX}) has no diagram"
        );
        let mine: Vec<&Block> = blocks
            .iter()
            .filter(|b| in_section(&b.section, OVERVIEW_PREFIX))
            .collect();
        assert!(
            !mine.is_empty(),
            "{lang}: the overview (section {OVERVIEW_PREFIX}) has no diagram"
        );
        let mut solid: BTreeSet<(String, String)> = BTreeSet::new();
        for b in &mine {
            assert!(
                b.entities.iter().all(|e| e.cols.is_empty()),
                "{lang} section {}: the overview shows relations only; an entity has attributes",
                b.section
            );
            for r in &b.rels {
                check_relation(lang, &b.section, r, &truth, &other);
                if r.solid {
                    solid.insert(pair(&r.a, &r.b));
                }
            }
        }
        for fk in &truth.fk_pairs {
            assert!(
                solid.contains(fk),
                "{lang} section {OVERVIEW_PREFIX}: the foreign key between `{}` and `{}` is missing as a solid relation",
                fk.0,
                fk.1
            );
        }
    }
}

#[test]
fn the_three_languages_agree() {
    let all = docs();
    let reference = signature(&all[0].1);
    for (lang, blocks) in all.iter().skip(1) {
        let sig = signature(blocks);
        for (entity, cols) in &reference.entities {
            match sig.entities.get(entity) {
                None => panic!("{lang}: entity `{entity}` is drawn in en but not in {lang}"),
                Some(other) if other != cols => {
                    for (i, (a, b)) in cols.iter().zip(other.iter()).enumerate() {
                        assert!(
                            a == b,
                            "{lang} entity `{entity}` column {}: en has `{} {}`, {lang} has `{} {}`",
                            i + 1,
                            a.1,
                            a.0,
                            b.1,
                            b.0
                        );
                    }
                    panic!(
                        "{lang} entity `{entity}`: en draws {} columns, {lang} draws {}",
                        cols.len(),
                        other.len()
                    );
                }
                Some(_) => {}
            }
        }
        for entity in sig.entities.keys() {
            assert!(
                reference.entities.contains_key(entity),
                "{lang}: entity `{entity}` is drawn in {lang} but not in en"
            );
        }
        assert_eq!(
            reference.rels, sig.rels,
            "{lang}: the relations of the diagrams differ from en (solid / dotted pairs per section)"
        );
    }
}

struct Signature {
    /// entity -> (column, type, marks as text) in order, over every block whose entity has attributes
    entities: BTreeMap<String, Vec<(String, String, String)>>,
    /// (section prefix of the diagram, "solid" | "dotted", unordered pair)
    rels: BTreeSet<(String, bool, (String, String))>,
}

fn signature(blocks: &[Block]) -> Signature {
    let mut entities = BTreeMap::new();
    let mut rels = BTreeSet::new();
    for b in blocks {
        for e in &b.entities {
            if !e.cols.is_empty() {
                let cols = e
                    .cols
                    .iter()
                    .map(|c| {
                        (
                            c.name.clone(),
                            c.ty.to_ascii_lowercase(),
                            c.marks.iter().cloned().collect::<Vec<_>>().join(","),
                        )
                    })
                    .collect();
                entities.insert(e.name.clone(), cols);
            }
        }
        for r in &b.rels {
            rels.insert((b.section.clone(), r.solid, pair(&r.a, &r.b)));
        }
    }
    Signature { entities, rels }
}
