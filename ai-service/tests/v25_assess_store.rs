//! v2.5 storage of the study assessment: a `summary` chat with an `assess@` ref is listed as the wire kind `assess`,
//! a record summary stays `summary`, and the saved lookup of a record never returns an assessment chat.
use xexam_ai_service::config::JournalMode;
use xexam_ai_service::identity::{Identity, Role};
use xexam_ai_service::store::data::{self, StudyInsert};
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

#[test]
fn assess_saved_matches_the_stored_line_and_the_owner() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    let line = "[AI assessment] latest=2026-11-21 10:30 30d 3r 6q";
    pair(
        &store,
        1,
        &insert("assess@2026-11-22", "2026-11-22", None, line),
        "advice",
    );
    let hit = store
        .read(|c| data::assess_saved(c, 1, "2026-11-21 10:30"))
        .expect("read")
        .expect("hit");
    assert_eq!(hit.date, "2026-11-22");
    assert_eq!(hit.line.records, 3);
    assert_eq!(hit.saved.latest.expect("latest").content, "advice");
    for (user, at) in [(1, "2026-11-21 10:31"), (2, "2026-11-21 10:30")] {
        let got = store
            .read(|c| data::assess_saved(c, user, at))
            .expect("read");
        assert!(got.is_none(), "{user} {at}");
    }
    // a set without any attempt has nothing saved
    let dir2 = tempfile::tempdir().expect("dir");
    let store2 = Store::open(dir2.path(), JournalMode::Delete).expect("open");
    assert!(store2
        .read(|c| data::assess_saved(c, 1, "2026-11-21 10:30"))
        .expect("read")
        .is_none());
}

/// Writes a pair and leaves the answer as `status` (with `text`), as a running, stopped or failed attempt.
fn attempt(
    store: &Store,
    user: i64,
    s: &StudyInsert<'_>,
    status: &str,
    text: &str,
) -> data::StudyIds {
    let ids = store
        .write(|tx| data::prepare_study(tx, &who(user), s, &mut |_| true))
        .expect("pair");
    store
        .write(|tx| {
            tx.execute(
                "UPDATE messages SET content = ?2, status = ?3 WHERE id = ?1",
                rusqlite::params![ids.answer, text, status],
            )
        })
        .expect("status");
    ids
}

const LINE_X: &str = "[AI assessment] latest=2026-11-20 09:00 30d 2r 4q";
const LINE_Y: &str = "[AI assessment] latest=2026-11-21 10:30 30d 3r 6q";
const AT_X: &str = "2026-11-20 09:00";
const AT_Y: &str = "2026-11-21 10:30";

#[test]
fn assess_saved_returns_a_running_stopped_or_failed_attempt_without_a_good_one() {
    for status in ["streaming", "stopped", "error"] {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
        let ids = attempt(
            &store,
            1,
            &insert("assess@2026-11-22", "2026-11-22", None, LINE_Y),
            status,
            "partial",
        );
        let hit = store
            .read(|c| data::assess_saved(c, 1, AT_Y))
            .expect("read")
            .unwrap_or_else(|| panic!("{status}: the attempt must be returned"));
        let latest = hit.saved.latest.expect("latest");
        assert_eq!(latest.message_id, ids.answer, "{status}");
        assert_eq!(latest.status, status);
        assert_eq!(latest.content, "partial");
        assert!(hit.saved.last_good.is_none(), "{status}");
        assert_eq!(hit.saved.chat_id, Some(ids.chat));
        assert_eq!(hit.date, "2026-11-22");
        assert_eq!(hit.line.latest_at, AT_Y);
        // another record set still has nothing
        assert!(store
            .read(|c| data::assess_saved(c, 1, AT_X))
            .expect("read")
            .is_none());
    }
}

#[test]
fn assess_saved_follows_a_new_attempt_and_keeps_the_last_good_answer_of_the_same_set() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    let done = pair(
        &store,
        1,
        &insert("assess@2026-11-22", "2026-11-22", None, LINE_Y),
        "advice",
    );
    let run = attempt(
        &store,
        1,
        &insert("assess@2026-11-22", "2026-11-22", None, LINE_Y),
        "streaming",
        "",
    );
    let hit = store
        .read(|c| data::assess_saved(c, 1, AT_Y))
        .expect("read")
        .expect("hit");
    assert_eq!(hit.saved.latest.expect("latest").message_id, run.answer);
    assert_eq!(
        hit.saved.last_good.expect("last good").message_id,
        done.answer
    );
    // a finished answer of another set is never offered as the last good one
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    pair(
        &store,
        1,
        &insert("assess@2026-11-22", "2026-11-22", None, LINE_X),
        "old advice",
    );
    let run = attempt(
        &store,
        1,
        &insert("assess@2026-11-22", "2026-11-22", None, LINE_Y),
        "stopped",
        "half",
    );
    let hit = store
        .read(|c| data::assess_saved(c, 1, AT_Y))
        .expect("read")
        .expect("hit");
    assert_eq!(hit.saved.latest.expect("latest").message_id, run.answer);
    assert!(hit.saved.last_good.is_none());
}

#[test]
fn assess_saved_finds_the_matching_chat_after_a_follow_up_touched_an_older_one() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    let a = pair(
        &store,
        1,
        &insert("assess@2026-11-21", "2026-11-21", None, LINE_X),
        "advice x",
    );
    let b = pair(
        &store,
        1,
        &insert("assess@2026-11-22", "2026-11-22", None, LINE_Y),
        "advice y",
    );
    // a follow-up in AI Chat inside the older chat makes it the most recently touched one
    store
        .write(|tx| {
            tx.execute(
                "INSERT INTO messages (chat_id, role, content, status, created_at, finished_at, kind, parent_id)
                 VALUES (?1, 'user', 'and then?', 'done', ?2, ?2, 'chat', ?3)",
                rusqlite::params![a.chat, NOW + 600, a.answer],
            )?;
            let q = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO messages (chat_id, role, content, status, created_at, finished_at, kind, parent_id)
                 VALUES (?1, 'assistant', 'more advice', 'done', ?2, ?2, 'chat', ?3)",
                rusqlite::params![a.chat, NOW + 600, q],
            )?;
            data::touch_chat(tx, a.chat, NOW + 600)
        })
        .expect("follow-up");
    let hit = store
        .read(|c| data::assess_saved(c, 1, AT_Y))
        .expect("read")
        .expect("hit y");
    assert_eq!(hit.saved.chat_id, Some(b.chat));
    assert_eq!(hit.date, "2026-11-22");
    assert_eq!(hit.saved.latest.expect("latest").content, "advice y");
    let hit = store
        .read(|c| data::assess_saved(c, 1, AT_X))
        .expect("read")
        .expect("hit x");
    assert_eq!(hit.saved.chat_id, Some(a.chat));
    assert_eq!(hit.date, "2026-11-21");
    assert_eq!(hit.saved.latest.expect("latest").content, "advice x");
}

#[test]
fn assess_saved_finds_an_older_answer_of_the_same_chat_after_the_newest_record_is_gone() {
    let dir = tempfile::tempdir().expect("dir");
    let store = Store::open(dir.path(), JournalMode::Delete).expect("open");
    let x = pair(
        &store,
        1,
        &insert("assess@2026-11-22", "2026-11-22", None, LINE_X),
        "advice x",
    );
    pair(
        &store,
        1,
        &insert("assess@2026-11-22", "2026-11-22", None, LINE_Y),
        "advice y",
    );
    // the record of Y was deleted: the newest record is X again
    let hit = store
        .read(|c| data::assess_saved(c, 1, AT_X))
        .expect("read")
        .expect("hit");
    let latest = hit.saved.latest.expect("latest");
    assert_eq!(latest.message_id, x.answer);
    assert_eq!(latest.content, "advice x");
    assert_eq!(hit.line.records, 2);
}
