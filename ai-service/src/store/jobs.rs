//! Generation and import jobs: rows of the `tasks` table with `kind = 'bank_gen'` (spec GJ-1, IJ-1). A job never stores
//! question text: the payload names the source banks, the counts and the settings it started with; an import's payload
//! also holds the file's name, type and size (never its text); the result holds the new bank's id and name.
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Map, Value};

/// The only job kind.
pub const KIND: &str = "bank_gen";

const COLS: &str = "id, user_id, state, payload, result, error_code, attempts, created_at, updated_at, job_key, goal, made,
     phase, charged, seen, started_at, finished_at,
     (SELECT COUNT(*) FROM gen_calls WHERE task_id = tasks.id),
     (SELECT COALESCE(SUM(tokens_in), 0) FROM gen_calls WHERE task_id = tasks.id),
     (SELECT COALESCE(SUM(tokens_out), 0) FROM gen_calls WHERE task_id = tasks.id),
     (SELECT COALESCE(SUM(tokens_cached), 0) FROM gen_calls WHERE task_id = tasks.id),
     (SELECT COALESCE(SUM(tokens_cache_write), 0) FROM gen_calls WHERE task_id = tasks.id)";

/// One job row.
#[derive(Debug, Clone)]
pub struct JobRow {
    pub id: i64,
    pub user_id: i64,
    pub state: String,
    pub payload: Value,
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub attempts: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub job_key: Option<String>,
    pub goal: i64,
    pub made: i64,
    pub phase: Option<String>,
    pub charged: bool,
    pub seen: bool,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// Exact sums of the job's own `gen_calls` rows (every call, recognition included; A-OWNER-67).
    pub usage_calls: i64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    /// Input tokens the provider read from its prompt cache (A-OWNER-71)
    pub tokens_cached: i64,
    /// Input tokens the provider wrote to its prompt cache (A-OWNER-74)
    pub tokens_cache_write: i64,
}

impl JobRow {
    pub fn active(&self) -> bool {
        self.state == "queued" || self.state == "running"
    }

    /// `"bank"`, `"scope"`, `"wrong"` or `"file"`.
    pub fn source(&self) -> &str {
        self.payload
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("bank")
    }

    pub fn is_file(&self) -> bool {
        self.source() == "file"
    }

    /// The Job object of the API (API-U1, API-U6).
    pub fn to_json(&self) -> Value {
        let p = &self.payload;
        let is_file = self.is_file();
        let active = self.active();
        let from: Vec<Value> = p
            .get("from")
            .and_then(Value::as_array)
            .map(|a| a.iter().take(10).cloned().collect())
            .unwrap_or_default();
        let from_count = p
            .get("fromCount")
            .and_then(Value::as_i64)
            .or_else(|| {
                p.get("from")
                    .and_then(Value::as_array)
                    .map(|a| a.len() as i64)
            })
            .unwrap_or(0);
        let phase = if active { self.phase.clone() } else { None };
        let confirm_by = if is_file && active && phase.as_deref() == Some("confirm") {
            p.get("confirmBy").cloned().unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let skipped = if is_file && self.state == "done" {
            self.result
                .as_ref()
                .and_then(|r| r.get("skipped"))
                .cloned()
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let error = match self.state.as_str() {
            "failed" => self.error_code.clone().map(Value::from),
            "cancelled" if self.error_code.as_deref() == Some("ai.import.expired") => {
                Some(Value::from("ai.import.expired"))
            }
            _ => None,
        };
        let mut m = Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("state".into(), json!(self.state));
        m.insert("phase".into(), json!(phase));
        m.insert("source".into(), json!(self.source()));
        m.insert(
            "name".into(),
            p.get("name").cloned().unwrap_or(Value::from("")),
        );
        m.insert("from".into(), Value::Array(from));
        m.insert("fromCount".into(), json!(from_count));
        m.insert(
            "file".into(),
            if is_file {
                p.get("file").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            },
        );
        m.insert(
            "mode".into(),
            if is_file {
                p.get("mode").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            },
        );
        m.insert(
            "read".into(),
            if is_file {
                p.get("read").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            },
        );
        m.insert("confirmBy".into(), confirm_by);
        m.insert(
            "errorArg".into(),
            if is_file {
                p.get("errorArg").cloned().unwrap_or(Value::Null)
            } else {
                Value::Null
            },
        );
        m.insert("skipped".into(), skipped);
        // A-OWNER-47 (file import): the questions found in the file and how many of them the import takes (the first ones);
        // the result (done) wins over the confirm-step payload; null when unknown (material mode, other jobs)
        for key in ["total", "taken", "approx"] {
            let v = if is_file {
                self.result
                    .as_ref()
                    .and_then(|r| r.get(key))
                    .or_else(|| p.get(key))
                    .cloned()
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            m.insert(key.into(), v);
        }
        // `found` is the same number as `total` (the name the front end reads); `approx` = "about N": the model's count
        // and the local count of answer choices disagree (A-OWNER-70)
        let found = m.get("total").cloned().unwrap_or(Value::Null);
        m.insert("found".into(), found);
        m.insert("made".into(), json!(self.made));
        m.insert("goal".into(), json!(self.goal));
        m.insert("calls".into(), json!(self.attempts));
        m.insert("createdAt".into(), json!(self.created_at));
        m.insert("startedAt".into(), json!(self.started_at));
        m.insert("finishedAt".into(), json!(self.finished_at));
        m.insert(
            "bankId".into(),
            self.result
                .as_ref()
                .and_then(|r| r.get("bankId"))
                .cloned()
                .unwrap_or(Value::Null),
        );
        m.insert("error".into(), error.unwrap_or(Value::Null));
        m.insert(
            "charged".into(),
            json!(self.charged && self.state != "failed"),
        );
        m.insert(
            "usage".into(),
            json!({"calls": self.usage_calls, "tokensIn": self.tokens_in, "tokensOut": self.tokens_out, "tokensCached": self.tokens_cached, "tokensCacheWrite": self.tokens_cache_write}),
        );
        Value::Object(m)
    }
}

fn row_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobRow> {
    let payload: String = r.get(3)?;
    let result: Option<String> = r.get(4)?;
    Ok(JobRow {
        id: r.get(0)?,
        user_id: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
        state: r.get(2)?,
        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
        result: result.and_then(|s| serde_json::from_str(&s).ok()),
        error_code: r.get(5)?,
        attempts: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
        job_key: r.get(9)?,
        goal: r.get(10)?,
        made: r.get(11)?,
        phase: r.get(12)?,
        charged: r.get::<_, i64>(13)? != 0,
        seen: r.get::<_, i64>(14)? != 0,
        started_at: r.get(15)?,
        finished_at: r.get(16)?,
        usage_calls: r.get(17)?,
        tokens_in: r.get(18)?,
        tokens_out: r.get(19)?,
        tokens_cached: r.get(20)?,
        tokens_cache_write: r.get(21)?,
    })
}

/// A job by id (any user).
pub fn get(c: &Connection, id: i64) -> rusqlite::Result<Option<JobRow>> {
    c.query_row(
        &format!("SELECT {COLS} FROM tasks WHERE id = ?1 AND kind = '{KIND}'"),
        [id],
        row_from,
    )
    .optional()
}

/// A job of this user (a foreign id answers like a missing one).
pub fn get_for_user(c: &Connection, user: i64, id: i64) -> rusqlite::Result<Option<JobRow>> {
    c.query_row(
        &format!("SELECT {COLS} FROM tasks WHERE id = ?1 AND user_id = ?2 AND kind = '{KIND}'"),
        params![id, user],
        row_from,
    )
    .optional()
}

/// The user's queued or running job (through `idx_tasks_active`).
pub fn active_for_user(c: &Connection, user: i64) -> rusqlite::Result<Option<JobRow>> {
    c.query_row(
        &format!(
            "SELECT {COLS} FROM tasks WHERE user_id = ?1 AND kind = '{KIND}' AND state IN ('queued', 'running')"
        ),
        [user],
        row_from,
    )
    .optional()
}

/// Units used today: charged jobs whose charge moment is today (SEC-9: the day a unit was charged).
pub fn quota_used(c: &Connection, user: i64, day_start: i64) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT COUNT(*) FROM tasks WHERE user_id = ?1 AND kind = 'bank_gen' AND charged = 1 AND charged_at >= ?2",
        params![user, day_start],
        |r| r.get(0),
    )
}

/// Finished jobs the user has not seen (done or failed; a cancelled import that expired), newest first, at most `limit`.
pub fn unseen_for_user(c: &Connection, user: i64, limit: i64) -> rusqlite::Result<Vec<JobRow>> {
    let mut st = c.prepare(&format!(
        "SELECT {COLS} FROM tasks WHERE user_id = ?1 AND kind = '{KIND}' AND seen = 0
           AND (state IN ('done', 'failed') OR (state = 'cancelled' AND error_code = 'ai.import.expired'))
         ORDER BY id DESC LIMIT ?2"
    ))?;
    let rows = st.query_map(params![user, limit], row_from)?;
    rows.collect()
}

/// A job to insert.
pub struct NewJob<'a> {
    pub user_id: i64,
    pub job_key: &'a str,
    pub goal: i64,
    pub payload: &'a Value,
    pub charged: bool,
    pub phase: Option<&'a str>,
    pub now: i64,
}

/// Why an insert failed.
#[derive(Debug)]
pub enum InsertError {
    /// The unique index `idx_tasks_active` refused it: the user already has an active job
    Busy,
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for InsertError {
    fn from(e: rusqlite::Error) -> InsertError {
        InsertError::Db(e)
    }
}

/// Inserts a queued job (the caller made sure the user's row exists).
pub fn insert(tx: &Transaction, j: &NewJob<'_>) -> Result<i64, InsertError> {
    let res = tx.execute(
        "INSERT INTO tasks (user_id, kind, state, payload, attempts, run_after, created_at, updated_at, job_key, goal, made,
                            phase, charged, seen, charged_at)
         VALUES (?1, 'bank_gen', 'queued', ?2, 0, ?3, ?3, ?3, ?4, ?5, 0, ?6, ?7, 0, ?8)",
        params![
            j.user_id,
            j.payload.to_string(),
            j.now,
            j.job_key,
            j.goal,
            j.phase,
            i64::from(j.charged),
            j.charged.then_some(j.now),
        ],
    );
    match res {
        Ok(_) => Ok(tx.last_insert_rowid()),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(InsertError::Busy)
        }
        Err(e) => Err(InsertError::Db(e)),
    }
}

/// state running, `started_at` = now when asked to (a generation starts when it gets a job place; an import's
/// `started_at` was set at its confirm).
pub fn mark_running(
    tx: &Transaction,
    id: i64,
    now: i64,
    set_started: bool,
) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE tasks SET state = 'running', started_at = CASE WHEN ?3 THEN ?2 ELSE started_at END, updated_at = ?2
         WHERE id = ?1 AND state IN ('queued', 'running')",
        params![id, now, set_started],
    )
}

/// One model call was sent: `attempts` + 1, the valid questions so far, the phase. Returns the rows changed (0 = the
/// task is gone: the user's AI data was purged).
pub fn after_call(
    tx: &Transaction,
    id: i64,
    made: Option<i64>,
    phase: Option<&str>,
    now: i64,
) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE tasks SET attempts = attempts + 1, made = COALESCE(?2, made), phase = COALESCE(?3, phase), updated_at = ?4
         WHERE id = ?1",
        params![id, made, phase, now],
    )
}

pub fn set_phase(tx: &Transaction, id: i64, phase: &str, now: i64) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE tasks SET phase = ?2, updated_at = ?3 WHERE id = ?1 AND state IN ('queued', 'running')",
        params![id, phase, now],
    )
}

/// Rewrites the payload (and the goal when given).
pub fn update_payload(
    tx: &Transaction,
    id: i64,
    payload: &Value,
    goal: Option<i64>,
    now: i64,
) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE tasks SET payload = ?2, goal = COALESCE(?3, goal), updated_at = ?4 WHERE id = ?1",
        params![id, payload.to_string(), goal, now],
    )
}

/// The confirm of an import: charged, phase writing, state queued again, `started_at` = now.
pub fn confirm(tx: &Transaction, id: i64, now: i64) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE tasks SET charged = 1, charged_at = ?2, phase = 'writing', state = 'queued', started_at = ?2, updated_at = ?2
         WHERE id = ?1 AND state IN ('queued', 'running') AND phase = 'confirm'",
        params![id, now],
    )
}

/// How a job ends.
pub struct End<'a> {
    /// `done`, `failed` or `cancelled`
    pub state: &'a str,
    pub error: Option<&'a str>,
    pub charged: bool,
    pub result: Option<&'a Value>,
    pub seen: bool,
}

/// Ends a job (only while it is queued or running). Returns the rows changed.
pub fn finish(tx: &Transaction, id: i64, e: &End<'_>, now: i64) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE tasks SET state = ?2, error_code = ?3, result = ?4, charged = ?5,
                          charged_at = CASE WHEN ?5 = 1 THEN charged_at ELSE NULL END,
                          seen = ?6, phase = NULL, finished_at = ?7, updated_at = ?7
         WHERE id = ?1 AND state IN ('queued', 'running')",
        params![
            id,
            e.state,
            e.error,
            e.result.map(Value::to_string),
            i64::from(e.charged),
            i64::from(e.seen),
            now
        ],
    )
}

/// Marks a finished job as seen (idempotent).
pub fn set_seen(tx: &Transaction, id: i64) -> rusqlite::Result<usize> {
    tx.execute("UPDATE tasks SET seen = 1 WHERE id = ?1", [id])
}

/// Restart repair: every active job (a job in the phase `saving` too unless `keep_saving`) becomes failed
/// `ai.gen.interrupted`, not charged. Returns the number repaired.
pub fn repair_interrupted(
    tx: &Transaction,
    now: i64,
    keep_saving: bool,
) -> rusqlite::Result<usize> {
    tx.execute(
        "UPDATE tasks SET state = 'failed', error_code = 'ai.gen.interrupted', charged = 0, charged_at = NULL,
                          phase = NULL, finished_at = ?1, updated_at = ?1
         WHERE kind = 'bank_gen' AND state IN ('queued', 'running') AND (?2 = 0 OR COALESCE(phase, '') <> 'saving')",
        params![now, i64::from(keep_saving)],
    )
}

/// Active jobs in the phase `saving` (the restart lookup by job key).
pub fn saving_jobs(c: &Connection) -> rusqlite::Result<Vec<JobRow>> {
    let mut st = c.prepare(&format!(
        "SELECT {COLS} FROM tasks WHERE kind = '{KIND}' AND state IN ('queued', 'running') AND phase = 'saving'"
    ))?;
    let rows = st.query_map([], row_from)?;
    rows.collect()
}

/// Every active job id (queued or running), for the runner registry's tests and the purge.
pub fn active_ids_of(c: &Connection, user: i64) -> rusqlite::Result<Vec<i64>> {
    let mut st = c.prepare(
        "SELECT id FROM tasks WHERE user_id = ?1 AND kind = 'bank_gen' AND state IN ('queued', 'running')",
    )?;
    let rows = st.query_map([user], |r| r.get(0))?;
    rows.collect()
}

/// A row of the administrators' list: the job, its user and its token sums.
pub struct AdminJob {
    pub job: JobRow,
    pub username: String,
}

/// Every user's jobs, newest first (`limit` rows): the active ones and the finished ones still kept (30 days).
pub fn list_admin(c: &Connection, limit: i64) -> rusqlite::Result<Vec<AdminJob>> {
    let mut st = c.prepare(&format!(
        "SELECT {COLS},
                (SELECT username FROM users WHERE id = tasks.user_id)
         FROM tasks WHERE kind = '{KIND}' ORDER BY id DESC LIMIT ?1"
    ))?;
    let rows = st.query_map([limit], |r| {
        Ok(AdminJob {
            job: row_from(r)?,
            username: r.get::<_, Option<String>>(22)?.unwrap_or_default(),
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::JournalMode;
    use crate::store::Store;

    const NOW: i64 = 1_700_000_000;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("dir");
        let s = Store::open(dir.path(), JournalMode::Delete).expect("open");
        s.write(|tx| {
            for id in [1, 2] {
                tx.execute(
                    "INSERT INTO users (id, stamp, username, first_seen_at, last_seen_at) VALUES (?1, 'x', ?2, 1, 1)",
                    params![id, format!("user{id}")],
                )?;
            }
            Ok::<_, rusqlite::Error>(())
        })
        .expect("users");
        (dir, s)
    }

    fn new_job<'a>(user: i64, key: &'a str, payload: &'a Value, charged: bool) -> NewJob<'a> {
        NewJob {
            user_id: user,
            job_key: key,
            goal: 20,
            payload,
            charged,
            phase: None,
            now: NOW,
        }
    }

    #[test]
    fn one_active_job_per_user_is_a_database_rule() {
        let (_d, s) = store();
        let p = json!({"source": "bank", "name": "n"});
        let first = s
            .write(|tx| insert(tx, &new_job(1, "k1", &p, true)))
            .expect("first");
        let second = s.write(|tx| insert(tx, &new_job(1, "k2", &p, true)));
        assert!(matches!(second, Err(InsertError::Busy)));
        // another user is fine, and the same user once the first ended
        s.write(|tx| insert(tx, &new_job(2, "k3", &p, true)))
            .expect("other user");
        s.write(|tx| {
            finish(
                tx,
                first,
                &End {
                    state: "done",
                    error: None,
                    charged: true,
                    result: Some(&json!({"bankId": 5})),
                    seen: false,
                },
                NOW + 1,
            )
        })
        .expect("finish");
        s.write(|tx| insert(tx, &new_job(1, "k4", &p, true)))
            .expect("after the first ended");
    }

    #[test]
    fn the_quota_counts_the_day_a_unit_was_charged() {
        let (_d, s) = store();
        let p = json!({"source": "bank"});
        let day_start = NOW - 3600;
        let id = s
            .write(|tx| insert(tx, &new_job(1, "k1", &p, true)))
            .expect("job");
        assert_eq!(s.read(|c| quota_used(c, 1, day_start)).expect("used"), 1);
        // a unit charged yesterday does not count today
        s.write(|tx| {
            tx.execute(
                "UPDATE tasks SET charged_at = ?1 WHERE id = ?2",
                params![day_start - 1, id],
            )
        })
        .expect("move");
        assert_eq!(s.read(|c| quota_used(c, 1, day_start)).expect("used"), 0);
        // a failed job is never charged
        s.write(|tx| {
            finish(
                tx,
                id,
                &End {
                    state: "failed",
                    error: Some("ai.gen.quality"),
                    charged: false,
                    result: None,
                    seen: false,
                },
                NOW,
            )
        })
        .expect("fail");
        let row = s.read(|c| get(c, id)).expect("get").expect("row");
        assert!(!row.charged);
        assert_eq!(s.read(|c| quota_used(c, 1, 0)).expect("used"), 0);
    }

    #[test]
    fn the_job_object_has_the_documented_keys_in_order() {
        let (_d, s) = store();
        let p = json!({"source": "bank", "name": "Kitchen basics_user1_20261112093000",
                       "from": [{"id": 3, "name": "Kitchen basics"}], "fromCount": 1});
        let id = s
            .write(|tx| insert(tx, &new_job(1, "k1", &p, true)))
            .expect("job");
        let row = s.read(|c| get(c, id)).expect("get").expect("row");
        let v = row.to_json();
        let keys: Vec<&String> = v.as_object().expect("object").keys().collect();
        assert_eq!(
            keys,
            vec![
                "id",
                "state",
                "phase",
                "source",
                "name",
                "from",
                "fromCount",
                "file",
                "mode",
                "read",
                "confirmBy",
                "errorArg",
                "skipped",
                "total",
                "taken",
                "approx",
                "found",
                "made",
                "goal",
                "calls",
                "createdAt",
                "startedAt",
                "finishedAt",
                "bankId",
                "error",
                "charged",
                "usage"
            ]
        );
        assert_eq!(v["state"], "queued");
        assert_eq!(v["charged"], true);
        assert_eq!(v["file"], Value::Null);
        assert_eq!(v["goal"], 20);
    }

    #[test]
    fn unseen_lists_done_failed_and_expired_imports_only() {
        let (_d, s) = store();
        let p = json!({"source": "bank"});
        let mut ids = Vec::new();
        for (i, (state, err)) in [
            ("done", None),
            ("failed", Some("ai.gen.quality")),
            ("cancelled", None),
            ("cancelled", Some("ai.import.expired")),
        ]
        .into_iter()
        .enumerate()
        {
            let key = format!("k{i}");
            let id = s
                .write(|tx| insert(tx, &new_job(1, &key, &p, false)))
                .expect("job");
            s.write(|tx| {
                finish(
                    tx,
                    id,
                    &End {
                        state,
                        error: err,
                        charged: false,
                        result: None,
                        seen: false,
                    },
                    NOW + 1,
                )
            })
            .expect("finish");
            ids.push(id);
        }
        let list = s.read(|c| unseen_for_user(c, 1, 3)).expect("unseen");
        let got: Vec<i64> = list.iter().map(|r| r.id).collect();
        assert_eq!(
            got,
            vec![ids[3], ids[1], ids[0]],
            "newest first, cancelled (not expired) left out"
        );
    }

    #[test]
    fn restart_repair_fails_active_jobs_and_can_keep_saving_ones() {
        let (_d, s) = store();
        let p = json!({"source": "bank"});
        let a = s
            .write(|tx| insert(tx, &new_job(1, "ka", &p, true)))
            .expect("a");
        let b = s
            .write(|tx| insert(tx, &new_job(2, "kb", &p, true)))
            .expect("b");
        s.write(|tx| set_phase(tx, b, "saving", NOW))
            .expect("phase");
        let n = s
            .write(|tx| repair_interrupted(tx, NOW + 5, true))
            .expect("repair");
        assert_eq!(n, 1);
        let ra = s.read(|c| get(c, a)).expect("a").expect("row");
        assert_eq!(ra.state, "failed");
        assert_eq!(ra.error_code.as_deref(), Some("ai.gen.interrupted"));
        assert!(!ra.charged);
        assert_eq!(s.read(saving_jobs).expect("saving").len(), 1);
        let n = s
            .write(|tx| repair_interrupted(tx, NOW + 6, false))
            .expect("repair all");
        assert_eq!(n, 1);
    }

    #[test]
    fn usage_sums_only_the_jobs_own_call_rows() {
        let (_d, s) = store();
        let p = json!({"source": "bank", "name": "n", "from": [], "fromCount": 0});
        let a = s
            .write(|tx| insert(tx, &new_job(1, "ka", &p, false)))
            .expect("a");
        let b = s
            .write(|tx| insert(tx, &new_job(2, "kb", &p, false)))
            .expect("b");
        s.write(|tx| {
            for (task, ti, to) in [(a, 100, 40), (a, 2500, 900), (b, 7, 7)] {
                tx.execute(
                    "INSERT INTO gen_calls (task_id, user_id, purpose, at, day, provider, tokens_in, tokens_out, latency_ms, status)
                     VALUES (?1, 1, 'import', 1, '2026-01-01', 'p', ?2, ?3, 5, 'ok')",
                    params![task, ti, to],
                )?;
            }
            // a call without token figures counts as a call and adds nothing
            tx.execute(
                "INSERT INTO gen_calls (task_id, user_id, purpose, at, day, provider, latency_ms, status)
                 VALUES (?1, 1, 'recognise', 1, '2026-01-01', 'p', 5, 'error')",
                [a],
            )?;
            Ok::<_, rusqlite::Error>(())
        })
        .expect("calls");
        let row = s
            .read(|c| get_for_user(c, 1, a))
            .expect("get")
            .expect("row");
        assert_eq!(
            row.to_json()["usage"],
            json!({"calls": 3, "tokensIn": 2600, "tokensOut": 940, "tokensCached": 0, "tokensCacheWrite": 0})
        );
        let none = s.read(|c| get_for_user(c, 2, a)).expect("get");
        assert!(none.is_none(), "a foreign job answers like a missing one");
        let fresh = s.read(|c| get(c, b)).expect("get").expect("row");
        assert_eq!(
            fresh.to_json()["usage"],
            json!({"calls": 1, "tokensIn": 7, "tokensOut": 7, "tokensCached": 0, "tokensCacheWrite": 0})
        );
    }

    #[test]
    fn the_quota_and_active_queries_use_their_indexes() {
        let (_d, s) = store();
        let plans: Vec<String> = s
            .read(|c| {
                let mut out = Vec::new();
                for sql in [
                    "SELECT COUNT(*) FROM tasks WHERE user_id = 1 AND kind = 'bank_gen' AND charged = 1 AND charged_at >= 5",
                    "SELECT id FROM tasks WHERE user_id = 1 AND kind = 'bank_gen' AND state IN ('queued', 'running')",
                    "SELECT id FROM tasks WHERE job_key = 'abc'",
                ] {
                    let mut st = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                    let rows = st.query_map([], |r| r.get::<_, String>(3))?;
                    out.push(rows.collect::<Result<Vec<_>, _>>()?.join(" | "));
                }
                Ok(out)
            })
            .expect("plans");
        assert!(plans[0].contains("idx_tasks_charged"), "{}", plans[0]);
        assert!(
            plans[1].contains("idx_tasks_active") || plans[1].contains("idx_tasks_user"),
            "{}",
            plans[1]
        );
        assert!(plans[2].contains("idx_tasks_job"), "{}", plans[2]);
    }

    #[test]
    fn a_task_update_after_a_purge_changes_no_row() {
        let (_d, s) = store();
        let p = json!({"source": "bank"});
        let id = s
            .write(|tx| insert(tx, &new_job(1, "k1", &p, true)))
            .expect("job");
        s.write(|tx| crate::store::data::delete_user(tx, 1))
            .expect("purge");
        let n = s
            .write(|tx| after_call(tx, id, Some(3), Some("writing"), NOW))
            .expect("update");
        assert_eq!(n, 0);
    }
}
