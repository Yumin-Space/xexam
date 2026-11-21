//! v1.9 storage: AI database schema 9 (fresh and upgraded files, the `study` slot split into `hint` + `explain`, the
//! per-connection switch, the pacing limits, the Gemma effort fold). All content is made up.
use rusqlite::Connection;
use std::path::Path;
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::store::conn::{self, NewConn, NewPool, SLOT_NAMES};
use xexam_ai_service::store::tables;
use xexam_ai_service::store::{schema, Store, DB_FILE};

const NOW: i64 = 1_700_000_000;

fn open(dir: &Path) -> Store {
    Store::open(dir, JournalMode::Delete).expect("open store")
}

fn raw(dir: &Path) -> Connection {
    Connection::open(dir.join(DB_FILE)).expect("raw")
}

fn schema_of(c: &Connection) -> String {
    c.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
        r.get(0)
    })
    .expect("schema")
}

fn has_column(c: &Connection, table: &str, column: &str) -> bool {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2)",
        [table, column],
        |r| r.get(0),
    )
    .expect("pragma")
}

/// (slot, model_id, updated_at, updated_by, effort)
type SlotRow = (String, Option<i64>, i64, String, Option<String>);

fn slot_rows(c: &Connection) -> Vec<SlotRow> {
    let mut st = c
        .prepare(
            "SELECT slot, model_id, updated_at, updated_by, effort FROM model_slots ORDER BY slot",
        )
        .expect("prepare");
    let rows = st
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("rows");
    rows
}

fn backups(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir.join("backups")) else {
        return Vec::new();
    };
    let mut v: Vec<String> = rd
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn new_conn(name: &str) -> NewConn<'_> {
    NewConn {
        name,
        provider: "openai",
        base_url: "https://api.example.com/v1",
        format: "openai",
        gateway_auth: "bearer",
        lists_models: true,
        effort_family: "openai",
        by: "root",
        now: NOW,
    }
}

/// Turns a fresh (schema 9) file into what a v1.8 build left: schema 8, the old `model_slots` with a `study` slot, none
/// of the four v1.9 columns.
fn make_v8(dir: &Path, study_model: Option<i64>, study_effort: Option<&str>) {
    let c = raw(dir);
    c.execute_batch(&format!(
        "DROP TABLE model_slots;
         CREATE TABLE model_slots (
           slot       TEXT PRIMARY KEY NOT NULL CHECK (slot IN ('default', 'chat', 'study', 'summary', 'generate', 'import')),
           model_id   INTEGER REFERENCES models(id) ON DELETE SET NULL,
           updated_at INTEGER NOT NULL,
           updated_by TEXT NOT NULL DEFAULT '',
           effort     TEXT
         );
         INSERT INTO model_slots (slot, model_id, updated_at) VALUES
           ('default', NULL, 5), ('chat', NULL, 5), ('summary', NULL, 5), ('generate', NULL, 5), ('import', NULL, 5);
         INSERT INTO model_slots (slot, model_id, updated_at, updated_by, effort)
           VALUES ('study', {}, 7, 'root', {});
         ALTER TABLE connections DROP COLUMN enabled;
         ALTER TABLE limits DROP COLUMN pace_on;
         ALTER TABLE limits DROP COLUMN pace_rpm;
         ALTER TABLE limits DROP COLUMN pace_tpm;
         UPDATE meta SET value = '8' WHERE key = 'schema';",
        study_model.map_or("NULL".to_string(), |m| m.to_string()),
        study_effort.map_or("NULL".to_string(), |e| format!("'{e}'")),
    ))
    .expect("v8 fixture");
}

#[test]
fn a_fresh_file_is_schema_9_with_seven_slots_and_the_new_columns() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    let c = raw(dir.path());
    assert_eq!(schema::SCHEMA_VERSION, "11");
    assert_eq!(schema_of(&c), "11");
    for (table, column, _) in schema::V9_COLUMNS {
        assert!(has_column(&c, table, column), "{table}.{column}");
    }
    let slots: Vec<String> = slot_rows(&c).into_iter().map(|r| r.0).collect();
    let mut want: Vec<String> = SLOT_NAMES.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(slots, want);
    assert!(!slots.iter().any(|s| s == "study"));
    assert!(backups(dir.path()).is_empty());
    // the pacing defaults come from the column defaults of a new file
    let l = Store::open(dir.path(), JournalMode::Delete)
        .expect("reopen")
        .read(tables::load_limits)
        .expect("limits");
    assert_eq!((l.pace_on, l.pace_rpm, l.pace_tpm), (1, 13, 230_000));
    // the slot CHECK refuses the removed name
    assert!(c
        .execute(
            "INSERT INTO model_slots (slot, updated_at) VALUES ('study', 1)",
            []
        )
        .is_err());
}

#[test]
fn a_schema_8_file_is_backed_up_and_study_becomes_hint_and_explain() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let (conn_id, model) = store
        .write(|tx| {
            let id = conn::insert_connection(tx, &new_conn("Garden"))?;
            let m = conn::upsert_pool(
                tx,
                &NewPool {
                    connection_id: id,
                    model: "gemini-demo",
                    manual: true,
                    status: "ok",
                    error_code: None,
                    usage_option: true,
                    max_tokens: 4096,
                    context: None,
                    latency_ms: None,
                    tested_at: None,
                },
            )?;
            Ok::<_, rusqlite::Error>((id, m))
        })
        .expect("conn + model");
    drop(store);
    make_v8(dir.path(), Some(model), Some("google:HIGH"));

    let store = open(dir.path());
    let c = raw(dir.path());
    assert_eq!(schema_of(&c), "11");
    let rows = slot_rows(&c);
    let get = |name: &str| rows.iter().find(|r| r.0 == name).cloned().expect(name);
    assert!(rows.iter().all(|r| r.0 != "study"));
    assert_eq!(rows.len(), 7);
    for name in ["hint", "explain"] {
        assert_eq!(
            get(name),
            (
                name.to_string(),
                Some(model),
                7,
                "root".to_string(),
                Some("google:HIGH".to_string())
            )
        );
    }
    assert_eq!(get("default").2, 5);
    assert_eq!(get("chat").1, None);
    // the connection is on, the limits carry the pacing defaults, no temporary table is left
    let on: i64 = c
        .query_row(
            "SELECT enabled FROM connections WHERE id = ?1",
            [conn_id],
            |r| r.get(0),
        )
        .expect("enabled");
    assert_eq!(on, 1);
    let l = store.read(tables::load_limits).expect("limits");
    assert_eq!((l.pace_on, l.pace_rpm, l.pace_tpm), (1, 13, 230_000));
    let left: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'model_slots_v8'",
            [],
            |r| r.get(0),
        )
        .expect("master");
    assert_eq!(left, 0);
    // the slots API agrees with the table
    assert_eq!(
        store.read(conn::slots).expect("slots"),
        [None, None, Some(model), Some(model), None, None, None]
    );
    // one backup, still schema 8
    let copies = backups(dir.path());
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert!(copies[0].starts_with("xexam-ai-schema8-") && copies[0].ends_with(".db"));
    let copy = Connection::open(dir.path().join("backups").join(&copies[0])).expect("copy");
    assert_eq!(schema_of(&copy), "8");
    assert!(has_column(&copy, "model_slots", "effort"));
    // a second start changes nothing and makes no second backup
    drop(store);
    drop(open(dir.path()));
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn an_upgraded_file_has_the_same_sqlite_master_as_a_new_one() {
    let master = |dir: &Path| -> Vec<(String, String, Option<String>)> {
        let c = raw(dir);
        let mut st = c
            .prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name")
            .expect("prepare");
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("rows");
        rows
    };
    let fresh = tempfile::tempdir().expect("dir");
    drop(open(fresh.path()));
    let old = tempfile::tempdir().expect("dir");
    drop(open(old.path()));
    make_v8(old.path(), None, None);
    drop(open(old.path()));
    // ALTER TABLE ADD COLUMN stores the added column text after the original DDL, so compare the table names + the
    // model_slots shape (the rebuilt table is created from the same text as the new file's)
    let names = |m: &[(String, String, Option<String>)]| -> Vec<(String, String)> {
        m.iter().map(|r| (r.0.clone(), r.1.clone())).collect()
    };
    assert_eq!(names(&master(fresh.path())), names(&master(old.path())));
    let slots_sql = |dir: &Path| {
        master(dir)
            .into_iter()
            .find(|r| r.1 == "model_slots")
            .and_then(|r| r.2)
            .expect("model_slots")
    };
    assert_eq!(
        slots_sql(fresh.path())
            .split_whitespace()
            .collect::<Vec<_>>(),
        slots_sql(old.path()).split_whitespace().collect::<Vec<_>>()
    );
}

#[test]
fn a_v9_file_missing_a_v9_column_gets_it_back_without_a_backup() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    raw(dir.path())
        .execute_batch("ALTER TABLE limits DROP COLUMN pace_rpm;")
        .expect("drop");
    drop(open(dir.path()));
    let c = raw(dir.path());
    assert!(has_column(&c, "limits", "pace_rpm"));
    assert_eq!(schema_of(&c), "11");
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn the_gemma_effort_choices_fold_into_google_on_every_start() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    {
        let c = raw(dir.path());
        for (slot, tag) in [
            ("default", "gemma:HIGH"),
            ("chat", "gemma:true"),
            ("hint", "gemma:MINIMAL"),
            ("explain", "gemma:false"),
            ("summary", "gemma:LOW"),
            ("generate", "anthropic:HIGH"),
        ] {
            c.execute(
                "UPDATE model_slots SET effort = ?2 WHERE slot = ?1",
                [slot, tag],
            )
            .expect("set");
        }
    }
    let store = open(dir.path());
    let efforts: Vec<(String, Option<String>)> = slot_rows(&raw(dir.path()))
        .into_iter()
        .map(|r| (r.0, r.4))
        .collect();
    let of = |slot: &str| efforts.iter().find(|r| r.0 == slot).expect(slot).1.clone();
    assert_eq!(of("default").as_deref(), Some("google:HIGH"));
    assert_eq!(of("chat").as_deref(), Some("google:HIGH"));
    assert_eq!(of("hint").as_deref(), Some("google:MINIMAL"));
    assert_eq!(of("explain").as_deref(), Some("google:MINIMAL"));
    assert_eq!(of("summary"), None);
    assert_eq!(of("generate").as_deref(), Some("anthropic:HIGH"));
    assert_eq!(of("import"), None);
    // the API reads every remaining value
    assert!(store.read(conn::efforts).is_ok());
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn a_gemma_choice_in_a_schema_8_file_folds_during_the_upgrade() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    // the slots resolve to a Gemini-model connection (family google), so the folded tag survives the 10 -> 11 step
    let model = store
        .write(|tx| {
            let id = conn::insert_connection(tx, &new_conn("Garden"))?;
            conn::upsert_pool(
                tx,
                &NewPool {
                    connection_id: id,
                    model: "gemma-demo",
                    manual: true,
                    status: "ok",
                    error_code: None,
                    usage_option: true,
                    max_tokens: 4096,
                    context: None,
                    latency_ms: None,
                    tested_at: None,
                },
            )
        })
        .expect("conn + model");
    drop(store);
    make_v8(dir.path(), Some(model), Some("gemma:true"));
    drop(open(dir.path()));
    let rows = slot_rows(&raw(dir.path()));
    for name in ["hint", "explain"] {
        let r = rows.iter().find(|r| r.0 == name).expect(name);
        assert_eq!(r.4.as_deref(), Some("google:HIGH"), "{name}");
    }
}

#[test]
fn global_hint_and_explain_bits_are_independent_and_never_re_paired() {
    let dir = tempfile::tempdir().expect("dir");
    drop(open(dir.path()));
    raw(dir.path())
        .execute("UPDATE settings SET features_off = 2 WHERE id = 1", [])
        .expect("global hint off");
    let store = open(dir.path());
    assert_eq!(store.read(conn::global_features_off).expect("read"), 2);
    // a schema-8 file: the global bits stay as stored, the per-user bits are still paired
    drop(store);
    make_v8(dir.path(), None, None);
    {
        let c = raw(dir.path());
        c.execute_batch(
            "UPDATE settings SET features_off = 4 WHERE id = 1;
             INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at, features_off)
               VALUES (100, 's0', 'demo-0', 1, 1, 2), (101, 's1', 'demo-1', 1, 1, 6);",
        )
        .expect("rows");
    }
    let store = open(dir.path());
    assert_eq!(store.read(conn::global_features_off).expect("read"), 4);
    let bits: Vec<i64> = {
        let c = raw(dir.path());
        let mut st = c
            .prepare("SELECT features_off FROM users WHERE id >= 100 ORDER BY id")
            .expect("prepare");
        let rows = st
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("rows");
        rows
    };
    assert_eq!(bits, vec![0, 6]);
}

#[test]
fn the_connection_switch_keeps_everything_and_does_not_bump_the_revision() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let id = store
        .write(|tx| conn::insert_connection(tx, &new_conn("Garden")))
        .expect("insert");
    let before = store
        .read(|c| conn::connection(c, id))
        .expect("read")
        .expect("row");
    assert!(before.enabled, "a new connection starts on");
    let found = store
        .write(|tx| conn::set_connection_enabled(tx, id, false, "root", NOW + 5))
        .expect("off");
    assert!(found);
    let off = store
        .read(|c| conn::connection(c, id))
        .expect("read")
        .expect("row");
    assert!(!off.enabled);
    assert_eq!(off.rev, before.rev, "no revision bump");
    assert_eq!((off.updated_at, off.updated_by.as_str()), (NOW + 5, "root"));
    assert_eq!(
        (
            off.name.as_str(),
            off.base_url.as_str(),
            off.status.as_str()
        ),
        ("Garden", "https://api.example.com/v1", "ok")
    );
    // idempotent, and back on
    assert!(store
        .write(|tx| conn::set_connection_enabled(tx, id, false, "root", NOW + 6))
        .expect("off again"));
    assert!(store
        .write(|tx| conn::set_connection_enabled(tx, id, true, "root", NOW + 7))
        .expect("on"));
    assert!(
        store
            .read(|c| conn::connection(c, id))
            .expect("read")
            .expect("row")
            .enabled
    );
    // an unknown id reports false
    assert!(!store
        .write(|tx| conn::set_connection_enabled(tx, id + 99, true, "root", NOW))
        .expect("unknown"));
    // the list carries the flag too
    let all = store.read(conn::connections).expect("list");
    assert_eq!(all.len(), 1);
    assert!(all[0].enabled);
}

#[test]
fn the_pacing_limits_round_trip_through_the_limits_row() {
    let dir = tempfile::tempdir().expect("dir");
    let store = open(dir.path());
    let body = serde_json::json!({"paceOn": 0, "paceRpm": 4000, "paceTpm": 12_345_678});
    let l = store
        .read(tables::load_limits)
        .expect("load")
        .apply_update(body.as_object().expect("object"))
        .expect("in range");
    store
        .write(|tx| tables::save_limits(tx, &l, "root", NOW))
        .expect("save");
    let back = store.read(tables::load_limits).expect("load");
    assert_eq!(back, l);
    assert_eq!(
        (back.pace_on, back.pace_rpm, back.pace_tpm),
        (0, 4000, 12_345_678)
    );
}
