//! v2.5 storage of the study assessment: a `summary` chat with an `assess@` ref is listed as the wire kind `assess`,
//! a record summary stays `summary`, and the saved lookup of a record never returns an assessment chat. v2.6: the
//! latest-assessment row (the Home capsule) is written by the finish hook only and lives apart from the chats.
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::identity::{Identity, Role};
use xexam_ai_service::store::data::{self, StudyInsert};
use xexam_ai_service::store::stats;
use xexam_ai_service::store::Store;

const NOW: i64 = 1_800_000_000;

fn who(id: i64) -> Identity {
    Identity {
        id,
        stamp: format!("{:032x}", 0x7000 + id),
        username: format!("user{id}"),
        role: Role::User,
        ai_allowed: true,
        sid: "0".repeat(64),
    }
}

fn insert<'a>(
    chat_ref: &'a str,
    title: &'a str,
    record: Option<i64>,
    line: &'a str,
) -> StudyInsert<'a> {
    StudyInsert {
        chat_kind: "summary",
        chat_ref,
        title,
        record_id: record,
        kind: "summary",
        question_id: None,
        question_no: None,
        source: None,
        lang: "en",
        sample: None,
        user_content: line,
        model: "demo-model",
        now: NOW,
        chats_per_user: 1000,
        messages_per_chat: 400,
    }
}

/// Writes a pair and finishes the answer with `text`.
fn pair(store: &Store, user: i64, s: &StudyInsert<'_>, text: &str) -> data::StudyIds {
    let ids = store
        .write(|tx| data::prepare_study(tx, &who(user), s, &mut |_| true))
        .expect("pair");
    store
        .write(|tx| {
            tx.execute(
                "UPDATE messages SET content = ?2, status = 'done' WHERE id = ?1",
                rusqlite::params![ids.answer, text],
            )
        })
        .expect("finish");
    ids
}

#[test]
fn an_assessment_lists_as_assess_and_a_record_summary_stays_summary() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    let a = pair(
        &store,
        1,
        &insert(
            "assess@2026-11-22",
            "2026-11-22",
            None,
            "[AI assessment] latest=2026-11-21 10:30 30d 3r 6q",
        ),
        "advice",
    );
    let s = pair(
        &store,
        1,
        &insert(
            "r7@2026-11-21 10:30",
            "2026-11-21 10:30",
            Some(7),
            "[AI summary] x",
        ),
        "summary text",
    );
    let chats = store.read(|c| data::list_chats(c, 1)).expect("chats");
    let kind_of = |id: i64| {
        chats
            .iter()
            .find(|c| c.id == id)
            .expect("chat")
            .kind
            .clone()
    };
    assert_eq!(kind_of(a.chat), "assess");
    assert_eq!(kind_of(s.chat), "summary");
    let one = store
        .read(|c| data::chat(c, 1, a.chat))
        .expect("chat")
        .expect("found");
    assert_eq!(one.kind, "assess");
    let msgs = store.read(|c| data::messages(c, a.chat)).expect("messages");
    assert!(msgs
        .iter()
        .all(|m| m.kind == "assess" && m.record_id.is_none()));
    let msgs = store.read(|c| data::messages(c, s.chat)).expect("messages");
    assert!(msgs
        .iter()
        .all(|m| m.kind == "summary" && m.record_id == Some(7)));
    // the raw rows keep the CHECK-safe values
    let raw: (String, String) = store
        .read(|c| {
            c.query_row(
                "SELECT kind, (SELECT kind FROM messages WHERE chat_id = chats.id LIMIT 1) FROM chats WHERE id = ?1",
                [a.chat],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .expect("raw");
    assert_eq!(raw, ("summary".to_string(), "summary".to_string()));
    // running info reports the wire kind
    let run = store
        .read(|c| data::running_info(c, 1, a.chat, a.answer))
        .expect("info")
        .expect("some");
    assert_eq!(run["chatKind"], "assess");
    assert_eq!(run["kind"], "assess");
    assert!(run["recordId"].is_null());
}

#[test]
fn the_record_summary_lookup_never_returns_an_assessment_chat() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    pair(
        &store,
        1,
        &insert(
            "assess@2026-11-22",
            "2026-11-22",
            None,
            "[AI assessment] latest=2026-11-21 10:30 30d 3r 6q",
        ),
        "advice",
    );
    for r in ["r7@2026-11-21 10:30", "assess@2026-11-22", "r1@2026-11-22"] {
        let got = store.read(|c| data::summary_saved(c, 1, r)).expect("saved");
        assert_eq!(got.chat_id.is_some(), r == "assess@2026-11-22", "{r}");
    }
    // by the record-summary key shape nothing is found
    let none = store
        .read(|c| data::summary_saved(c, 1, "r3@2026-11-21 10:30"))
        .expect("saved");
    assert!(none.chat_id.is_none());
}

const LINE: &str = "[AI assessment] latest=2026-11-21 10:30 30d 3r 6q";
const DAY: &str = "2026-11-22";

fn start(store: &Store, user: i64, day_ref: &str) -> data::StudyIds {
    store
        .write(|tx| {
            data::prepare_study(
                tx,
                &who(user),
                &insert(day_ref, "t", None, LINE),
                &mut |_| true,
            )
        })
        .expect("pair")
}

/// What the finish hook calls for one answer.
fn finish(store: &Store, answer: i64, status: &str, text: &str, _day: &str) -> bool {
    store
        .write(|tx| {
            tx.execute(
                "UPDATE messages SET content = ?2, status = ?3 WHERE id = ?1",
                rusqlite::params![answer, text, status],
            )?;
            data::save_assess_latest(tx, answer, status, text, "demo-model", NOW)
        })
        .expect("finish")
}

fn latest(store: &Store, user: i64) -> Option<data::AssessLatest> {
    store.read(|c| data::assess_latest(c, user)).expect("read")
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    (dir, store)
}

#[test]
fn a_done_assessment_writes_the_capsule_with_the_numbers_of_its_line() {
    let (_d, store) = open();
    let ids = start(&store, 1, "assess@2026-11-22");
    assert!(finish(&store, ids.answer, "done", "Keep going.", DAY));
    let got = latest(&store, 1).expect("row");
    assert_eq!(got.content, "Keep going.");
    assert_eq!(got.lang, "en");
    assert_eq!(got.model.as_deref(), Some("demo-model"));
    assert_eq!(got.latest_at, "2026-11-21 10:30");
    assert_eq!((got.records, got.answered), (3, 6));
    assert_eq!((got.day.as_str(), got.created_at), (DAY, NOW));
    // another user has none
    assert!(latest(&store, 2).is_none());
    // a newer done answer replaces it (one row per user)
    let again = start(&store, 1, "assess@2026-11-23");
    assert!(finish(
        &store,
        again.answer,
        "done",
        "Next day.",
        "2026-11-23"
    ));
    let got = latest(&store, 1).expect("row");
    assert_eq!(
        (got.content.as_str(), got.day.as_str()),
        ("Next day.", "2026-11-23")
    );
    let rows: i64 = store
        .read(|c| c.query_row("SELECT COUNT(*) FROM assess_latest", [], |r| r.get(0)))
        .expect("count");
    assert_eq!(rows, 1);
}

#[test]
fn failed_stopped_and_empty_answers_write_nothing_and_keep_the_old_text() {
    let (_d, store) = open();
    let first = start(&store, 1, "assess@2026-11-22");
    assert!(finish(&store, first.answer, "done", "Old text.", DAY));
    for (status, text) in [
        ("error", "x"),
        ("stopped", "partial"),
        (
            "done", "  
",
        ),
        ("done", ""),
    ] {
        let ids = start(&store, 1, "assess@2026-11-23");
        assert!(
            !finish(&store, ids.answer, status, text, "2026-11-23"),
            "{status}"
        );
        let got = latest(&store, 1).expect("row");
        assert_eq!(
            (got.content.as_str(), got.day.as_str()),
            ("Old text.", DAY),
            "{status}"
        );
    }
    // a fresh user whose only attempt fails has no capsule
    let ids = start(&store, 2, "assess@2026-11-22");
    assert!(!finish(&store, ids.answer, "error", "", DAY));
    assert!(latest(&store, 2).is_none());
}

#[test]
fn other_answers_never_write_a_capsule() {
    let (_d, store) = open();
    // a record summary is a summary chat without the assess@ ref
    let s = pair(
        &store,
        1,
        &insert("r7@2026-11-21 10:30", "x", Some(7), "[AI summary] x"),
        "text",
    );
    assert!(!finish(&store, s.answer, "done", "text", DAY));
    // an assess chat whose stored line cannot be read writes nothing
    let bad = store
        .write(|tx| {
            data::prepare_study(
                tx,
                &who(1),
                &insert("assess@2026-11-22", "t", None, "not a line"),
                &mut |_| true,
            )
        })
        .expect("pair");
    assert!(!finish(&store, bad.answer, "done", "text", DAY));
    assert!(latest(&store, 1).is_none());
}

#[test]
fn the_capsule_survives_chat_deletes_and_the_retention_sweep_but_not_the_user() {
    let (_d, store) = open();
    let ids = start(&store, 1, "assess@2026-11-22");
    finish(&store, ids.answer, "done", "Keep going.", DAY);
    // one chat deleted
    assert_eq!(
        store
            .write(|tx| data::delete_chat(tx, 1, ids.chat))
            .expect("del"),
        1
    );
    assert_eq!(latest(&store, 1).expect("row").content, "Keep going.");
    // delete all
    let again = start(&store, 1, "assess@2026-11-23");
    assert!(again.new_chat);
    store
        .write(|tx| data::delete_all_chats(tx, 1))
        .expect("all");
    let chats: i64 = store
        .read(|c| c.query_row("SELECT COUNT(*) FROM chats", [], |r| r.get(0)))
        .expect("count");
    assert_eq!(chats, 0);
    assert_eq!(latest(&store, 1).expect("row").content, "Keep going.");
    // the retention sweep removes the old chat but not the capsule
    let third = start(&store, 1, "assess@2026-11-24");
    let swept = store
        .write(|tx| stats::sweep_chats(tx, NOW + 1_000_000, &[], 100))
        .expect("sweep");
    assert_eq!(swept, 1, "{}", third.chat);
    assert_eq!(latest(&store, 1).expect("row").content, "Keep going.");
    // deleting the user removes it (cascade)
    assert!(store.write(|tx| data::delete_user(tx, 1)).expect("user"));
    assert!(latest(&store, 1).is_none());
}

#[test]
fn the_table_is_added_to_an_older_file_on_open_without_a_version_change() {
    let dir = tempfile::tempdir().expect("dir");
    {
        let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
        store
            .write(|tx| tx.execute_batch("DROP TABLE assess_latest"))
            .expect("drop");
    }
    let store = Store::open(dir.path(), JournalMode::Delete).expect("reopen");
    assert!(latest(&store, 1).is_none());
    let version: String = store
        .read(|c| {
            c.query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| {
                r.get(0)
            })
        })
        .expect("schema");
    assert_eq!(version, "11");
}

#[test]
fn the_capsule_day_is_the_day_it_was_asked_not_the_day_it_ended() {
    let (_d, store) = open();
    let ids = start(&store, 1, "assess@2026-11-21");
    // the answer ends after midnight: the finish call no longer carries a day, the chat ref does
    assert!(finish(&store, ids.answer, "done", "Late.", "2026-11-22"));
    assert_eq!(latest(&store, 1).expect("row").day, "2026-11-21");
}

// —— v2.6: per-day count, usage mark, the `assess` slot upgrade ——————————————————————————————————————————————————————

#[test]
fn runs_count_per_day_and_restart_on_a_new_day() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Wal).expect("open");
    store
        .write(|tx| {
            tx.execute(
                "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (1, 'a', 'u', 1, 1)",
                [],
            )
        })
        .expect("user");
    let line = "[AI assessment] latest=2026-11-21 10:30 30d 3r 6q";
    let run = |day: &str, n: i64| {
        let r = format!("assess@{day}");
        let ins = insert(&r, day, None, line);
        let ids = pair(&store, 1, &ins, &format!("text {n}"));
        store
            .write(|tx| {
                // a text that passes the answer checks (a hidden capsule does not count as a run)
                let good = format!("A small step taken each day adds up like drops filling a quiet pond. You have answered a good number of questions lately and most were right. Next week, spend a short session on the ones you missed. Go at your own pace, run {n}.");
                data::save_assess_latest(tx, ids.answer, "done", &good, "m", NOW)
            })
            .expect("save")
    };
    assert!(run("2026-11-22", 1));
    assert!(run("2026-11-22", 2));
    let l = store
        .read(|c| data::assess_latest(c, 1))
        .expect("read")
        .expect("row");
    assert_eq!((l.day.as_str(), l.runs), ("2026-11-22", 2));
    assert!(run("2026-11-23", 3));
    let l = store
        .read(|c| data::assess_latest(c, 1))
        .expect("read")
        .expect("row");
    assert_eq!((l.day.as_str(), l.runs), ("2026-11-23", 1));
}

#[test]
fn an_assess_slot_is_added_to_an_older_file_as_a_copy_of_summary() {
    let dir = tempfile::tempdir().expect("dir");
    {
        let store = Store::open(dir.path(), JournalMode::Wal).expect("open");
        drop(store);
    }
    let path = dir.path().join(xexam_ai_service::store::DB_FILE);
    {
        let c = rusqlite::Connection::open(&path).expect("raw");
        c.execute_batch(
            "INSERT INTO connections (name, provider, base_url, created_at, updated_at) VALUES ('c', 'openai', 'http://x', 1, 1);
             INSERT INTO models (connection_id, model) VALUES (1, 'm1');
             UPDATE model_slots SET model_id = 1, effort = 'openai:high', updated_by = 'root' WHERE slot = 'summary';
             ALTER TABLE model_slots RENAME TO model_slots_new;
             CREATE TABLE model_slots (
               slot TEXT PRIMARY KEY NOT NULL CHECK (slot IN ('default', 'chat', 'hint', 'explain', 'summary', 'generate', 'import')),
               model_id INTEGER REFERENCES models(id) ON DELETE SET NULL, updated_at INTEGER NOT NULL,
               updated_by TEXT NOT NULL DEFAULT '', effort TEXT);
             INSERT INTO model_slots SELECT slot, model_id, updated_at, updated_by, effort FROM model_slots_new WHERE slot <> 'assess';
             DROP TABLE model_slots_new;",
        )
        .expect("older shape");
    }
    let store = Store::open(dir.path(), JournalMode::Wal).expect("reopen");
    let rows: Vec<(String, Option<i64>, Option<String>)> = store
        .read(|c| {
            let mut st = c.prepare("SELECT slot, model_id, effort FROM model_slots WHERE slot IN ('summary', 'assess') ORDER BY slot")?;
            let r = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            r.collect()
        })
        .expect("rows");
    assert_eq!(
        rows,
        vec![
            (
                "assess".to_string(),
                Some(1),
                Some("openai:high".to_string())
            ),
            (
                "summary".to_string(),
                Some(1),
                Some("openai:high".to_string())
            ),
        ]
    );
    // idempotent
    drop(store);
    let store = Store::open(dir.path(), JournalMode::Wal).expect("again");
    let n: i64 = store
        .read(|c| c.query_row("SELECT COUNT(*) FROM model_slots", [], |r| r.get(0)))
        .expect("n");
    assert_eq!(n, 8);
}
