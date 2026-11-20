//! v2.1 AI database (schema 11): `connections.effort_family` and `chats.pick_effort`. The 10 -> 11 upgrade infers the
//! family of every connection, clears slot strengths of another family, backs the file up first and logs ids only; a
//! new file and an upgraded one are the same. All content is made up.
mod common;

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::effort::EffortFamily;
use xexam_ai_service::store::conn::{self, ConnChange, NewConn};
use xexam_ai_service::store::{data, schema, Store, DB_FILE};

fn open(dir: &Path) -> Store {
    Store::open(dir, JournalMode::Delete).expect("open store")
}

fn raw(dir: &Path) -> Connection {
    Connection::open(dir.join(DB_FILE)).expect("open file")
}

fn has_column(c: &Connection, table: &str, column: &str) -> bool {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        [table, column],
        |r| r.get(0),
    )
    .expect("pragma")
}

fn schema_of(c: &Connection) -> String {
    c.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
        r.get(0)
    })
    .expect("schema")
}

fn backups(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir.join("backups"))
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn master(c: &Connection) -> Vec<(String, String, Option<String>)> {
    let mut st = c
        .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
        .expect("prepare");
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query");
    rows.collect::<Result<_, _>>().expect("rows")
}

fn family_of(c: &Connection, id: i64) -> String {
    c.query_row(
        "SELECT effort_family FROM connections WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .expect("family")
}

fn slot_effort(c: &Connection, slot: &str) -> Option<String> {
    c.query_row(
        "SELECT effort FROM model_slots WHERE slot = ?1",
        [slot],
        |r| r.get(0),
    )
    .expect("effort")
}

/// A schema-11 file turned into a v2.0 (schema 10) one: the two new columns removed, the stamp moved back.
fn downgrade_to_10(dir: &Path) {
    let c = raw(dir);
    c.execute_batch(
        "ALTER TABLE connections DROP COLUMN effort_family;
         ALTER TABLE chats DROP COLUMN pick_effort;
         UPDATE meta SET value = '10' WHERE key = 'schema';",
    )
    .expect("downgrade");
}

fn add_conn(c: &Connection, id: i64, provider: &str, format: &str, responses: i64) {
    c.execute(
        "INSERT INTO connections (id, name, provider, base_url, format, responses, sort, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'https://api.example.test/v1', ?4, ?5, ?1, 1, 1)",
        rusqlite::params![id, format!("c{id}"), provider, format, responses],
    )
    .expect("connection");
}

fn add_model(c: &Connection, id: i64, conn_id: i64, model: &str) {
    c.execute(
        "INSERT INTO models (id, connection_id, model) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, conn_id, model],
    )
    .expect("model");
}

fn set_slot(c: &Connection, slot: &str, model: Option<i64>, effort: Option<&str>) {
    c.execute(
        "UPDATE model_slots SET model_id = ?2, effort = ?3 WHERE slot = ?1",
        rusqlite::params![slot, model, effort],
    )
    .expect("slot");
}

/// A schema-11 file with seven made-up connections, a pool, slots and one chat pick, still carrying the new columns.
fn seeded(dir: &Path) {
    drop(open(dir));
    let c = raw(dir);
    add_conn(&c, 1, "anthropic", "anthropic", 0);
    add_conn(&c, 2, "gateway", "anthropic", 0);
    add_conn(&c, 3, "openai", "openai", 1);
    add_conn(&c, 4, "openai", "openai", 0);
    add_conn(&c, 5, "gateway", "openai", 0);
    add_conn(&c, 6, "openai", "openai", 0);
    add_conn(&c, 7, "openai", "openai", 0);
    add_model(&c, 10, 1, "claude-test-1");
    add_model(&c, 11, 3, "gpt-test-1");
    // 4: two of three ids are gemini / gemma after normalizing -> google
    add_model(&c, 12, 4, "Models/Gemini-Test-Pro");
    add_model(&c, 13, 4, "vendor/gemma-test-3");
    add_model(&c, 14, 4, "gpt-test-2");
    // 5: a tie -> openai
    add_model(&c, 15, 5, "gemini-test-flash");
    add_model(&c, 16, 5, "gpt-test-3");
    // 6: no model at all -> openai; 7: only a chat pick names a gemini model -> google
    c.execute(
        "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at, features_off)
         VALUES (1, 's', 'u1', 1, 1, 0)",
        [],
    )
    .expect("user");
    c.execute(
        "INSERT INTO chats (id, user_id, title, created_at, updated_at, pick_conn, pick_model)
         VALUES (1, 1, 'chat', 1, 1, 7, 'gemini-test-lite')",
        [],
    )
    .expect("chat");
    // default -> connection 4 (google); chat -> connection 1 (anthropic); summary -> connection 3 (responses)
    set_slot(&c, "default", Some(12), Some("google:HIGH"));
    set_slot(&c, "chat", Some(10), Some("anthropic:high"));
    set_slot(&c, "hint", None, Some("openai:high")); // follows the default (google) -> cleared
    set_slot(&c, "explain", None, Some("google:LOW")); // follows the default (google) -> kept
    set_slot(&c, "summary", Some(11), Some("openai:low")); // connection 3 is responses -> cleared
    set_slot(&c, "generate", Some(10), Some("garbage")); // unreadable -> cleared
    drop(c);
    downgrade_to_10(dir);
}

#[test]
fn a_fresh_file_is_schema_11_with_the_two_columns() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let c = raw(dir.path());
    assert_eq!(schema::SCHEMA_VERSION, "11");
    assert_eq!(schema_of(&c), "11");
    assert!(has_column(&c, "connections", "effort_family"));
    assert!(has_column(&c, "chats", "pick_effort"));
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn the_ten_to_eleven_upgrade_infers_families_clears_slots_and_backs_up_first() {
    let dir = tempfile::tempdir().expect("dir");
    seeded(dir.path());
    drop(open(dir.path()));
    let c = raw(dir.path());
    assert_eq!(schema_of(&c), "11");
    for (id, want) in [
        (1, "anthropic"),
        (2, "anthropic"),
        (3, "responses"),
        (4, "google"),
        (5, "openai"),
        (6, "openai"),
        (7, "google"),
    ] {
        assert_eq!(family_of(&c, id), want, "connection {id}");
    }
    assert_eq!(slot_effort(&c, "default").as_deref(), Some("google:HIGH"));
    assert_eq!(slot_effort(&c, "chat").as_deref(), Some("anthropic:high"));
    assert_eq!(slot_effort(&c, "hint"), None);
    assert_eq!(slot_effort(&c, "explain").as_deref(), Some("google:LOW"));
    assert_eq!(slot_effort(&c, "summary"), None);
    assert_eq!(slot_effort(&c, "generate"), None);
    // the chat pick is untouched and the new level column starts empty
    let pick: (i64, String, Option<String>) = c
        .query_row(
            "SELECT pick_conn, pick_model, pick_effort FROM chats WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .expect("pick");
    assert_eq!(pick, (7, "gemini-test-lite".to_string(), None));
    // one backup, taken before the change: it is schema 10 and has neither column
    let b = backups(dir.path());
    assert_eq!(b.len(), 1);
    assert!(b[0]
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("xexam-ai-schema10-")));
    let old = Connection::open(&b[0]).expect("backup");
    assert_eq!(schema_of(&old), "10");
    assert!(!has_column(&old, "connections", "effort_family"));
    assert_eq!(slot_effort(&old, "hint").as_deref(), Some("openai:high"));
}

#[test]
fn reopening_a_schema_11_file_writes_nothing_and_takes_no_backup() {
    let dir = tempfile::tempdir().expect("dir");
    seeded(dir.path());
    drop(open(dir.path()));
    let before = master(&raw(dir.path()));
    let fam: Vec<String> = (1..=7).map(|i| family_of(&raw(dir.path()), i)).collect();
    // a hand-set family must survive the next start (the inference is number-guarded)
    raw(dir.path())
        .execute(
            "UPDATE connections SET effort_family = 'anthropic' WHERE id = 6",
            [],
        )
        .expect("set");
    drop(open(dir.path()));
    let c = raw(dir.path());
    assert_eq!(master(&c), before);
    assert_eq!(family_of(&c, 6), "anthropic");
    assert_eq!(family_of(&c, 4), fam[3]);
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn an_upgraded_file_has_the_same_sqlite_master_as_a_new_one() {
    let fresh = tempfile::tempdir().expect("dir");
    drop(open(fresh.path()));
    let up = tempfile::tempdir().expect("dir");
    drop(open(up.path()));
    downgrade_to_10(up.path());
    drop(open(up.path()));
    let (a, b) = (master(&raw(up.path())), master(&raw(fresh.path())));
    for (x, y) in a.iter().zip(b.iter()) {
        if x != y {
            let (xs, ys) = (
                x.2.clone().unwrap_or_default(),
                y.2.clone().unwrap_or_default(),
            );
            let at = xs
                .chars()
                .zip(ys.chars())
                .take_while(|(p, q)| p == q)
                .count();
            panic!(
                "{} differs at {at}: {:?} vs {:?}",
                x.1,
                &xs[at.saturating_sub(40)..],
                &ys[at.saturating_sub(40)..]
            );
        }
    }
    assert_eq!(a.len(), b.len());
}

#[test]
fn a_schema_11_file_missing_a_new_column_gets_it_back_without_a_backup() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    raw(dir.path())
        .execute_batch("ALTER TABLE chats DROP COLUMN pick_effort;")
        .expect("drop");
    drop(open(dir.path()));
    let c = raw(dir.path());
    assert!(has_column(&c, "chats", "pick_effort"));
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn inference_rules_and_the_family_check() {
    assert_eq!(
        conn::infer_family("anthropic", "anthropic", &[]),
        EffortFamily::Anthropic
    );
    assert_eq!(
        conn::infer_family("gateway", "anthropic", &["gemini-x"]),
        EffortFamily::Anthropic
    );
    assert_eq!(
        conn::infer_family("gateway", "responses", &["gemini-x"]),
        EffortFamily::Responses
    );
    assert_eq!(
        conn::infer_family("openai", "openai", &[]),
        EffortFamily::OpenAi
    );
    // the same id under two spellings counts once
    assert_eq!(
        conn::infer_family(
            "openai",
            "openai",
            &["gemini-x", "models/GEMINI-X", "gemma-y", "gpt-x"]
        ),
        EffortFamily::Google
    );
    assert_eq!(
        conn::infer_family("openai", "openai", &["gemini-x", "gpt-x"]),
        EffortFamily::OpenAi
    );

    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let new = |family: &'static str| NewConn {
        name: "made-up",
        provider: "openai",
        base_url: "https://api.example.test/v1",
        format: "openai",
        gateway_auth: "bearer",
        lists_models: true,
        effort_family: family,
        by: "t",
        now: 5,
    };
    assert!(store
        .write(|tx| conn::insert_connection(tx, &new("Google")))
        .is_err());
    let id = store
        .write(|tx| conn::insert_connection(tx, &new("google")))
        .expect("insert");
    let row = store
        .read(|c| conn::connection(c, id))
        .expect("read")
        .expect("row");
    assert_eq!(row.effort_family, "google");
    let change = |family: &'static str| ConnChange {
        name: "made-up",
        provider: "openai",
        base_url: "https://api.example.test/v1",
        format: "openai",
        gateway_auth: "bearer",
        lists_models: true,
        effort_family: family,
        by: "t",
        now: 6,
    };
    assert!(store
        .write(|tx| conn::update_connection(tx, id, row.rev, &change("nope")))
        .is_err());
    assert!(store
        .write(|tx| conn::update_connection(tx, id, row.rev, &change("responses")))
        .expect("update"));
    let row = store
        .read(|c| conn::connection(c, id))
        .expect("read")
        .expect("row");
    assert_eq!(row.effort_family, "responses");
}

#[test]
fn the_chat_level_is_stored_cleared_and_independent_of_the_model_pick() {
    let dir = tempfile::tempdir().expect("dir");
    seeded(dir.path());
    let store = open(dir.path());
    let level = |s: &Store| s.read(|c| data::chat(c, 1, 1)).expect("chat").expect("row");
    assert_eq!(level(&store).pick_effort, None);
    store
        .write(|tx| data::set_chat_pick_effort(tx, 1, Some("high")))
        .expect("set");
    let row = level(&store);
    assert_eq!(row.pick_effort.as_deref(), Some("high"));
    // clearing the model pick leaves the level; clearing the level leaves the pick
    store
        .write(|tx| data::set_chat_pick(tx, 1, None))
        .expect("pick");
    assert_eq!(level(&store).pick_effort.as_deref(), Some("high"));
    store
        .write(|tx| data::set_chat_pick_effort(tx, 1, None))
        .expect("clear");
    assert_eq!(level(&store).pick_effort, None);
}

#[test]
fn a_strength_of_a_slot_that_resolves_to_no_connection_is_cleared() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    {
        let c = raw(dir.path());
        add_conn(&c, 1, "openai", "openai", 0);
        set_slot(&c, "chat", None, Some("openai:low"));
        set_slot(&c, "hint", None, Some("garbage"));
    }
    downgrade_to_10(dir.path());
    drop(open(dir.path()));
    let c = raw(dir.path());
    assert_eq!(slot_effort(&c, "chat"), None);
    assert_eq!(slot_effort(&c, "hint"), None);
}

fn pace_of(c: &Connection) -> (i64, i64) {
    c.query_row("SELECT pace_rpm, pace_tpm FROM limits", [], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .expect("pace")
}

#[test]
fn the_ten_to_eleven_upgrade_converts_the_pace_values_once() {
    // still on the old defaults -> the new defaults
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    raw(dir.path())
        .execute("UPDATE limits SET pace_rpm = 15, pace_tpm = 250000", [])
        .expect("old defaults");
    downgrade_to_10(dir.path());
    drop(open(dir.path()));
    assert_eq!(pace_of(&raw(dir.path())), (13, 240_000));
    // reopening changes nothing
    drop(open(dir.path()));
    assert_eq!(pace_of(&raw(dir.path())), (13, 240_000));
    // any other value -> floor(value * 0.9), never below the column minimum
    for ((rpm, tpm), want) in [
        ((20, 100_000), (18, 90_000)),
        ((1, 1000), (1, 1000)),
        ((14, 250_000), (12, 240_000)),
    ] {
        let dir = tempfile::tempdir().expect("dir");
        drop(open(dir.path()));
        raw(dir.path())
            .execute("UPDATE limits SET pace_rpm = ?1, pace_tpm = ?2", [rpm, tpm])
            .expect("set");
        downgrade_to_10(dir.path());
        drop(open(dir.path()));
        assert_eq!(pace_of(&raw(dir.path())), want, "{rpm} / {tpm}");
    }
}
