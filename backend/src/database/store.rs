//! `Store`：与 `app/db.py` 的 `Store` 语义一致。
//!
//! - 一个连接放在 `std::sync::Mutex` 里（对应旧版的 `RLock`）；锁中毒时用 `into_inner` 恢复，
//!   并且在取锁时顺手把"上一个调用 panic 后遗留的未提交事务"回滚掉。
//! - 写操作一律 `BEGIN IMMEDIATE … COMMIT`，任何失败（包括 COMMIT 失败）都 `ROLLBACK`。
//! - `apply_import` 的预览计划在**同一个写事务内**重新计算（修复旧版的 T4：计划与实际写入之间可被插队）。
use super::legacy_progress::{self, AttemptRow, StatRow};
use super::pyjson;
use super::schema::{SCHEMA, SCHEMA_VERSION};
use super::timefmt::{Clock, SystemClock};
use super::{HISTORY_LIMIT, MASTER_STREAK};
use crate::error::DbError;
use crate::models::{Attempt, ImportRecord, ParsedSheet, Question, SkippedRow};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::de::Error as _;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// `IN (?,…)` 一次最多放多少个占位符（SQLite 的默认上限是 32766，留足余量）。
const IN_CHUNK: usize = 500;

pub struct Store {
    conn: Mutex<Connection>,
    clock: Arc<dyn Clock>,
}

fn json_err(msg: &str) -> DbError {
    DbError::Json(serde_json::Error::custom(msg.to_string()))
}

fn invalid_input(msg: &str) -> DbError {
    DbError::Sqlite(rusqlite::Error::ToSqlConversionFailure(
        msg.to_string().into(),
    ))
}

/// `round(100 * good / total)`，Python 的"银行家舍入"（四舍六入五成双），全程整数运算。
/// `total <= 0` 或 `good < 0` 时返回 0（旧版在 `total == 0` 时是 ZeroDivisionError，API 层先拦掉了）。
pub fn percent_round_half_even(good: i64, total: i64) -> i64 {
    if total <= 0 || good < 0 {
        return 0;
    }
    let num = i128::from(good) * 100;
    let tot = i128::from(total);
    let q = num / tot;
    let r = num % tot;
    let twice = r * 2;
    let up = twice > tot || (twice == tot && q % 2 == 1);
    i64::try_from(if up { q + 1 } else { q }).unwrap_or(i64::MAX)
}

/// 失败要回滚的事务。`conn` 的锁由调用者持有。
fn run_tx<T>(
    conn: &Connection,
    f: impl FnOnce(&Connection) -> Result<T, DbError>,
) -> Result<T, DbError> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match f(conn) {
        Ok(v) => match conn.execute_batch("COMMIT") {
            Ok(()) => Ok(v),
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(DbError::Sqlite(e))
            }
        },
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>, DbError> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key=?1", params![key], |r| {
            r.get::<_, String>(0)
        })
        .optional()?)
}

fn has_progress(conn: &Connection) -> Result<bool, DbError> {
    let n: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM stats) + (SELECT COUNT(*) FROM attempts) n",
        [],
        |r| r.get(0),
    )?;
    Ok(n != 0)
}

struct StoredQuestion {
    id: String,
    sheet: String,
    row_no: i64,
    data: String,
    active: bool,
}

fn load_questions(conn: &Connection, sql: &str) -> Result<Vec<StoredQuestion>, DbError> {
    let mut st = conn.prepare(sql)?;
    let mut rows = st.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(StoredQuestion {
            id: r.get(0)?,
            sheet: r.get(1)?,
            row_no: r.get(2)?,
            data: r.get(3)?,
            active: r.get::<_, i64>(4)? != 0,
        });
    }
    Ok(out)
}

fn parse_data(data: &str) -> Result<Map<String, Value>, DbError> {
    match serde_json::from_str::<Value>(data)? {
        Value::Object(m) => Ok(m),
        _ => Err(json_err("question data is not a JSON object")),
    }
}

fn drop_key(v: Value, key: &str) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.into_iter().filter(|(k, _)| k != key).collect()),
        other => other,
    }
}

/// 旧版 `_q_hash`：去掉 `row`，键排序，`ensure_ascii=False` 的 JSON 文本。
fn question_hash(q: &Question) -> Result<String, DbError> {
    let v = serde_json::to_value(q)?;
    Ok(pyjson::canonical(&drop_key(v, "row"))?)
}

fn stored_hash(r: &StoredQuestion) -> Result<String, DbError> {
    let mut obj = parse_data(&r.data)?;
    obj.insert("id".to_string(), Value::String(r.id.clone()));
    obj.insert("sheet".to_string(), Value::String(r.sheet.clone()));
    obj.insert("row".to_string(), Value::from(r.row_no));
    Ok(pyjson::canonical(&drop_key(Value::Object(obj), "row"))?)
}

/// 存进 `questions.data` 的 JSON：除 id / sheet / row 之外的全部字段，保持 `Question` 的声明顺序。
fn question_data_json(q: &Question) -> Result<String, DbError> {
    let Value::Object(m) = serde_json::to_value(q)? else {
        return Err(json_err("question did not serialise to an object"));
    };
    let data: Map<String, Value> = m
        .into_iter()
        .filter(|(k, _)| k != "id" && k != "sheet" && k != "row")
        .collect();
    Ok(pyjson::to_string(&Value::Object(data))?)
}

struct SheetAgg {
    name: String,
    total: i64,
    usable: i64,
    skipped: Vec<SkippedRow>,
    added: i64,
    updated: i64,
    removed: i64,
    restored: i64,
}

impl SheetAgg {
    fn new(name: &str, total: i64, usable: i64, skipped: Vec<SkippedRow>) -> Self {
        SheetAgg {
            name: name.to_string(),
            total,
            usable,
            skipped,
            added: 0,
            updated: 0,
            removed: 0,
            restored: 0,
        }
    }
    fn to_value(&self) -> Result<Value, DbError> {
        Ok(json!({
            "name": self.name,
            "total": self.total,
            "usable": self.usable,
            "skipped": serde_json::to_value(&self.skipped)?,
            "added": self.added,
            "updated": self.updated,
            "removed": self.removed,
            "restored": self.restored,
        }))
    }
}

struct Aggregator {
    aggs: Vec<SheetAgg>,
    pos: HashMap<String, usize>,
}

impl Aggregator {
    fn new(sheets: &[ParsedSheet]) -> Self {
        let mut a = Aggregator {
            aggs: Vec::new(),
            pos: HashMap::new(),
        };
        for s in sheets {
            let agg = SheetAgg::new(&s.name, s.total, s.usable, s.skipped.clone());
            match a.pos.get(&s.name).copied() {
                Some(i) => {
                    if let Some(slot) = a.aggs.get_mut(i) {
                        *slot = agg;
                    }
                }
                None => {
                    a.pos.insert(s.name.clone(), a.aggs.len());
                    a.aggs.push(agg);
                }
            }
        }
        a
    }
    /// 取（必要时新建）某个工作表的统计，旧版对"只有下架题的工作表"用 `setdefault` 新建。
    fn entry(&mut self, name: &str) -> Option<&mut SheetAgg> {
        let i = match self.pos.get(name).copied() {
            Some(i) => i,
            None => {
                let i = self.aggs.len();
                self.aggs.push(SheetAgg::new(name, 0, 0, Vec::new()));
                self.pos.insert(name.to_string(), i);
                i
            }
        };
        self.aggs.get_mut(i)
    }
}

struct PlanOut {
    json: Value,
    added: i64,
    updated: i64,
    restored: i64,
    removed: i64,
    skipped: i64,
}

fn len_i64(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

fn count_stats_for(conn: &Connection, ids: &[&str]) -> Result<i64, DbError> {
    let mut total = 0i64;
    for chunk in ids.chunks(IN_CHUNK) {
        let marks = vec!["?"; chunk.len()].join(",");
        let sql = format!("SELECT COUNT(*) c FROM stats WHERE question_id IN ({marks})");
        let n: i64 =
            conn.query_row(&sql, rusqlite::params_from_iter(chunk.iter()), |r| r.get(0))?;
        total += n;
    }
    Ok(total)
}

/// 旧版 `plan_import`：只读。
fn compute_plan(
    conn: &Connection,
    sheets: &[ParsedSheet],
    questions: &[Question],
) -> Result<PlanOut, DbError> {
    let rows = load_questions(
        conn,
        "SELECT id, sheet, row_no, data, active FROM questions",
    )?;
    let index: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    let new_ids: HashSet<&str> = questions.iter().map(|q| q.id.as_str()).collect();

    let (mut added, mut updated, mut restored): (Vec<&Question>, Vec<&Question>, Vec<&Question>) =
        (Vec::new(), Vec::new(), Vec::new());
    let mut unchanged = 0i64;
    for q in questions {
        match index.get(q.id.as_str()).and_then(|&i| rows.get(i)) {
            None => added.push(q),
            Some(r) if !r.active => restored.push(q),
            Some(r) => {
                if stored_hash(r)? != question_hash(q)? {
                    updated.push(q);
                } else {
                    unchanged += 1;
                }
            }
        }
    }
    let removed: Vec<&StoredQuestion> = rows
        .iter()
        .filter(|r| r.active && !new_ids.contains(r.id.as_str()))
        .collect();

    let mut agg = Aggregator::new(sheets);
    for q in &added {
        if let Some(a) = agg.entry(&q.sheet) {
            a.added += 1;
        }
    }
    for q in &updated {
        if let Some(a) = agg.entry(&q.sheet) {
            a.updated += 1;
        }
    }
    for q in &restored {
        if let Some(a) = agg.entry(&q.sheet) {
            a.restored += 1;
        }
    }
    for r in &removed {
        if let Some(a) = agg.entry(&r.sheet) {
            a.removed += 1;
        }
    }

    let mut samples = Vec::new();
    for r in removed.iter().take(5) {
        let stem = parse_data(&r.data)?
            .get("stem")
            .and_then(Value::as_str)
            .map(|s| s.chars().take(40).collect::<String>())
            .unwrap_or_default();
        samples.push(Value::String(stem));
    }
    let removed_ids: Vec<&str> = removed.iter().map(|r| r.id.as_str()).collect();
    let removed_stats = count_stats_for(conn, &removed_ids)?;

    let sheet_values = agg
        .aggs
        .iter()
        .map(SheetAgg::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let skipped: i64 = sheets.iter().map(|s| len_i64(s.skipped.len())).sum();
    let out = PlanOut {
        json: Value::Null,
        added: len_i64(added.len()),
        updated: len_i64(updated.len()),
        restored: len_i64(restored.len()),
        removed: len_i64(removed.len()),
        skipped,
    };
    let json = json!({
        "sheets": sheet_values,
        "added": out.added,
        "updated": out.updated,
        "unchanged": unchanged,
        "restored": out.restored,
        "removed": out.removed,
        "skipped": out.skipped,
        "usable": len_i64(questions.len()),
        "removed_samples": samples,
        "removed_stats": removed_stats,
    });
    Ok(PlanOut { json, ..out })
}

fn last_import(conn: &Connection) -> Result<Option<ImportRecord>, DbError> {
    Ok(import_rows(conn, 1)?.into_iter().next())
}

fn import_rows(conn: &Connection, limit: i64) -> Result<Vec<ImportRecord>, DbError> {
    let mut st = conn.prepare(
        "SELECT id, at, filename, added, updated, removed, restored, skipped, total FROM imports ORDER BY id DESC LIMIT ?1",
    )?;
    let mut rows = st.query(params![limit])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(ImportRecord {
            id: r.get(0)?,
            at: r.get(1)?,
            filename: r.get(2)?,
            added: r.get(3)?,
            updated: r.get(4)?,
            removed: r.get(5)?,
            restored: r.get(6)?,
            skipped: r.get(7)?,
            total: r.get(8)?,
        });
    }
    Ok(out)
}

fn progress_from(conn: &Connection) -> Result<Value, DbError> {
    let mut questions = Map::new();
    {
        let mut st = conn
            .prepare("SELECT question_id, attempts, correct, wrong, streak, last, at FROM stats")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let mut rec = Map::new();
            rec.insert("attempts".to_string(), Value::from(r.get::<_, i64>(1)?));
            rec.insert("correct".to_string(), Value::from(r.get::<_, i64>(2)?));
            rec.insert("wrong".to_string(), Value::from(r.get::<_, i64>(3)?));
            rec.insert("streak".to_string(), Value::from(r.get::<_, i64>(4)?));
            if let Some(last) = r.get::<_, Option<i64>>(5)? {
                rec.insert("last".to_string(), Value::Bool(last != 0));
            }
            if let Some(at) = r.get::<_, Option<String>>(6)?.filter(|a| !a.is_empty()) {
                rec.insert("at".to_string(), Value::String(at));
            }
            questions.insert(id, Value::Object(rec));
        }
    }
    let mut history = Vec::new();
    {
        let mut st =
            conn.prepare("SELECT at, sheets, mode, total, correct, percent, seconds FROM attempts ORDER BY id DESC LIMIT ?1")?;
        let mut rows = st.query(params![HISTORY_LIMIT])?;
        while let Some(r) = rows.next()? {
            let sheets: Value = serde_json::from_str(&r.get::<_, String>(1)?)?;
            history.push(json!({
                "at": r.get::<_, String>(0)?,
                "sheets": sheets,
                "mode": r.get::<_, String>(2)?,
                "total": r.get::<_, i64>(3)?,
                "correct": r.get::<_, i64>(4)?,
                "percent": r.get::<_, i64>(5)?,
                "seconds": r.get::<_, i64>(6)?,
            }));
        }
    }
    Ok(json!({
        "version": 1,
        "questions": Value::Object(questions),
        "history": history,
        "masterStreak": MASTER_STREAK,
    }))
}

impl Store {
    /// 打开（必要时创建）数据库并建表。对已有数据库**不改动内容**。时间用系统本地时间（尊重 `TZ`）。
    pub fn open(path: &Path) -> Result<Store, DbError> {
        Self::open_with_clock(path, Arc::new(SystemClock))
    }

    /// 同 [`Store::open`]，但时间由调用者提供（测试用）。
    pub fn open_with_clock(path: &Path, clock: Arc<dyn Clock>) -> Result<Store, DbError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                    Some(format!("cannot create data directory: {}", e.kind())),
                )
            })?;
        }
        // 不用 URI 解析，路径按字面量处理
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)?;
        conn.busy_timeout(Duration::from_millis(5000))?;
        // 不用 WAL（Docker Desktop 挂载 Windows 目录时共享内存不可靠）；单人使用，数据安全优先
        conn.pragma_update_and_check(None, "journal_mode", "DELETE", |r| r.get::<_, String>(0))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(SCHEMA)?;
        // 只在缺失时写入；先查再写，已有数据库的文件内容保持不变
        if get_meta(&conn, "schema")?.is_none() {
            conn.execute(
                "INSERT OR IGNORE INTO meta(key,value) VALUES(?1,?2)",
                params!["schema", SCHEMA_VERSION],
            )?;
        }
        Ok(Store {
            conn: Mutex::new(conn),
            clock,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        let g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if !g.is_autocommit() {
            // 上一个持锁者 panic 了，留下一个没结束的事务
            let _ = g.execute_batch("ROLLBACK");
        }
        g
    }

    fn tx<T>(&self, f: impl FnOnce(&Connection) -> Result<T, DbError>) -> Result<T, DbError> {
        let g = self.lock();
        run_tx(&g, f)
    }

    pub fn count_active(&self) -> Result<i64, DbError> {
        let g = self.lock();
        Ok(
            g.query_row("SELECT COUNT(*) c FROM questions WHERE active=1", [], |r| {
                r.get(0)
            })?,
        )
    }

    /// `/api/bank`：键顺序 file, mtime, error, questions, sheets（见 123_after_import_bank.json）
    pub fn bank(&self) -> Result<Value, DbError> {
        let (rows, last) = {
            let g = self.lock();
            (
                load_questions(
                    &g,
                    "SELECT id, sheet, row_no, data, active FROM questions ORDER BY seq",
                )?,
                last_import(&g)?,
            )
        };
        let mut order: Vec<(String, i64, i64)> = Vec::new();
        let mut pos: HashMap<String, usize> = HashMap::new();
        let mut questions = Vec::new();
        for r in &rows {
            let i = match pos.get(&r.sheet).copied() {
                Some(i) => i,
                None => {
                    order.push((r.sheet.clone(), 0, 0));
                    pos.insert(r.sheet.clone(), order.len() - 1);
                    order.len() - 1
                }
            };
            if let Some(entry) = order.get_mut(i) {
                if r.active {
                    entry.1 += 1;
                } else {
                    entry.2 += 1;
                }
            }
            if r.active {
                let mut q = parse_data(&r.data)?;
                q.insert("id".to_string(), Value::String(r.id.clone()));
                q.insert("sheet".to_string(), Value::String(r.sheet.clone()));
                q.insert("row".to_string(), Value::from(r.row_no));
                questions.push(Value::Object(q));
            }
        }
        let sheets: Vec<Value> = order
            .iter()
            .filter(|(_, usable, inactive)| *usable != 0 || *inactive != 0)
            .map(|(name, usable, inactive)| json!({"name": name, "usable": usable, "inactive": inactive, "total": usable, "skipped": []}))
            .collect();
        Ok(json!({
            "file": last.as_ref().map(|l| l.filename.as_str()).unwrap_or(""),
            "mtime": last.as_ref().map(|l| l.at.as_str()).unwrap_or(""),
            "error": "",
            "questions": questions,
            "sheets": sheets,
        }))
    }

    /// 预览：只读，不写库。
    pub fn plan_import(
        &self,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Value, DbError> {
        let g = self.lock();
        Ok(compute_plan(&g, sheets, questions)?.json)
    }

    /// 真正写入：一个事务（BEGIN IMMEDIATE … COMMIT，失败 ROLLBACK）。计划在事务里重新计算，返回值与 `plan_import` 同形。
    pub fn apply_import(
        &self,
        filename: &str,
        sheets: &[ParsedSheet],
        questions: &[Question],
    ) -> Result<Value, DbError> {
        let t = self.clock.now();
        self.tx(|c| {
            let plan = compute_plan(c, sheets, questions)?;
            let mut new_ids: HashSet<&str> = HashSet::with_capacity(questions.len());
            {
                let mut up = c.prepare(
                    "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at) VALUES(?1,?2,?3,?4,?5,1,?6,?7) \
                     ON CONFLICT(id) DO UPDATE SET sheet=excluded.sheet, row_no=excluded.row_no, seq=excluded.seq, \
                     data=excluded.data, active=1, updated_at=excluded.updated_at",
                )?;
                for (seq, q) in questions.iter().enumerate() {
                    new_ids.insert(q.id.as_str());
                    let data = question_data_json(q)?;
                    up.execute(params![q.id, q.sheet, q.row, len_i64(seq), data, t, t])?;
                }
            }
            // 上传的文件里没有的题 -> 下架（学习记录保留）
            let base = questions.len();
            let active_ids: Vec<String> = {
                let mut st = c.prepare("SELECT id FROM questions WHERE active=1")?;
                let rows = st.query_map([], |r| r.get::<_, String>(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            let gone: Vec<&String> = active_ids.iter().filter(|id| !new_ids.contains(id.as_str())).collect();
            {
                let mut down = c.prepare("UPDATE questions SET active=0, seq=?1, updated_at=?2 WHERE id=?3")?;
                for (i, qid) in gone.iter().enumerate() {
                    down.execute(params![len_i64(base + i), t, qid])?;
                }
            }
            c.execute(
                "INSERT INTO imports(at, filename, added, updated, removed, restored, skipped, total) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![t, filename, plan.added, plan.updated, plan.removed, plan.restored, plan.skipped, len_i64(questions.len())],
            )?;
            Ok(plan.json)
        })
    }

    pub fn import_history(&self, limit: i64) -> Result<Vec<ImportRecord>, DbError> {
        let g = self.lock();
        import_rows(&g, limit)
    }

    /// `/api/progress`：`{"version":1,"questions":{…},"history":[…],"masterStreak":2}`
    pub fn progress(&self) -> Result<Value, DbError> {
        let g = self.lock();
        progress_from(&g)
    }

    pub fn record_attempt(&self, attempt: &Attempt) -> Result<Value, DbError> {
        let total = len_i64(attempt.results.len());
        if total == 0 {
            return Err(invalid_input("an attempt needs at least one result"));
        }
        let good = len_i64(attempt.results.iter().filter(|r| r.correct).count());
        let percent = percent_round_half_even(good, total);
        let sheets_json = pyjson::to_string(&attempt.sheets)?;
        let t = self.clock.now();
        let g = self.lock();
        run_tx(&g, |c| {
            {
                let mut ins = c.prepare("INSERT OR IGNORE INTO stats(question_id) VALUES(?1)")?;
                let mut ok = c.prepare(
                    "UPDATE stats SET attempts=attempts+1, correct=correct+1, streak=streak+1, last=1, at=?1 WHERE question_id=?2",
                )?;
                let mut bad = c
                    .prepare("UPDATE stats SET attempts=attempts+1, wrong=wrong+1, streak=0, last=0, at=?1 WHERE question_id=?2")?;
                for r in &attempt.results {
                    ins.execute(params![r.id])?;
                    if r.correct {
                        ok.execute(params![t, r.id])?;
                    } else {
                        bad.execute(params![t, r.id])?;
                    }
                }
            }
            c.execute(
                "INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![t, sheets_json, attempt.mode, total, good, percent, attempt.seconds],
            )?;
            // 只保留最近的 HISTORY_LIMIT 场
            c.execute(
                "DELETE FROM attempts WHERE id NOT IN (SELECT id FROM attempts ORDER BY id DESC LIMIT ?1)",
                params![HISTORY_LIMIT],
            )?;
            Ok(())
        })?;
        progress_from(&g)
    }

    pub fn reset_progress(&self) -> Result<Value, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            c.execute("DELETE FROM stats", [])?;
            c.execute("DELETE FROM attempts", [])?;
            Ok(())
        })?;
        progress_from(&g)
    }

    /// 旧版 progress.json 迁移。返回 (是否迁移了, 说明)。格式不对时返回 (false, 说明)，不崩溃。
    pub fn migrate_legacy_progress(&self, path: &Path) -> Result<(bool, String), DbError> {
        let g = self.lock();
        if get_meta(&g, "legacy_migrated")?.is_some_and(|v| !v.is_empty()) {
            return Ok((false, "之前已经迁移过".to_string()));
        }
        if !path.is_file() {
            return Ok((false, "没有找到旧的 progress.json".to_string()));
        }
        let (qs, hist) = match legacy_progress::load(path) {
            Ok(x) => x,
            Err(e) => return Ok((false, format!("旧的 progress.json 读不了：{e}"))),
        };
        if has_progress(&g)? {
            return Ok((false, "数据库里已经有学习记录，没有覆盖".to_string()));
        }
        let (stats, attempts) = match legacy_progress::convert(&qs, &hist) {
            Ok(x) => x,
            Err(e) => {
                return Ok((
                    false,
                    format!("旧的 progress.json 里有格式不对的内容，没有迁移：{e}"),
                ))
            }
        };
        let t = self.clock.now();
        let res = run_tx(&g, |c| {
            if has_progress(c)? {
                return Ok(false);
            }
            write_legacy_rows(c, &stats, &attempts, &t)?;
            Ok(true)
        });
        match res {
            Ok(true) => Ok((
                true,
                format!("迁移了 {} 场成绩、{} 道题的答题记录", hist.len(), qs.len()),
            )),
            Ok(false) => Ok((false, "数据库里已经有学习记录，没有覆盖".to_string())),
            Err(DbError::Sqlite(rusqlite::Error::SqliteFailure(e, _)))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Ok((
                    false,
                    "旧的 progress.json 里有格式不对的内容，没有迁移：数据不满足表的约束"
                        .to_string(),
                ))
            }
            Err(e) => Err(e),
        }
    }
}

fn write_legacy_rows(
    c: &Connection,
    stats: &[StatRow],
    attempts: &[AttemptRow],
    t: &str,
) -> Result<(), DbError> {
    {
        let mut st = c.prepare(
            "INSERT OR REPLACE INTO stats(question_id, attempts, correct, wrong, streak, last, at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        )?;
        for s in stats {
            st.execute(params![
                s.id, s.attempts, s.correct, s.wrong, s.streak, s.last, s.at
            ])?;
        }
    }
    {
        let mut at = c.prepare("INSERT INTO attempts(at, sheets, mode, total, correct, percent, seconds) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
        for a in attempts {
            at.execute(params![
                a.at, a.sheets, a.mode, a.total, a.correct, a.percent, a.seconds
            ])?;
        }
    }
    c.execute(
        "INSERT INTO meta(key,value) VALUES('legacy_migrated', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![t],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::timefmt::FixedClock;
    use crate::models::{Answer, AttemptResult, Kind};

    fn question(sheet: &str, id: &str, stem: &str, row: i64) -> Question {
        Question {
            kind: Kind::Single,
            tag: String::new(),
            stem: stem.to_string(),
            options: vec![],
            items: vec![],
            answer: Answer::Letters(vec!["A".to_string()]),
            translation: String::new(),
            zh: None,
            explanation: String::new(),
            mnemonic: String::new(),
            note: String::new(),
            id: id.to_string(),
            sheet: sheet.to_string(),
            row,
        }
    }

    fn sheet(name: &str, usable: i64) -> ParsedSheet {
        ParsedSheet {
            name: name.to_string(),
            total: usable,
            usable,
            skipped: vec![],
        }
    }

    fn open_tmp() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open_with_clock(
            &dir.path().join("x.db"),
            Arc::new(FixedClock("2026-01-02 03:04".to_string())),
        )
        .expect("open");
        (dir, store)
    }

    #[test]
    fn store_is_send_and_sync() {
        fn assert_ss<T: Send + Sync>() {}
        assert_ss::<Store>();
    }

    #[test]
    fn percent_ties_go_to_even() {
        assert_eq!(percent_round_half_even(1, 8), 12);
        assert_eq!(percent_round_half_even(3, 8), 38);
        assert_eq!(percent_round_half_even(1, 40), 2);
        assert_eq!(percent_round_half_even(1, 200), 0);
        assert_eq!(percent_round_half_even(0, 5), 0);
        assert_eq!(percent_round_half_even(5, 5), 100);
        assert_eq!(percent_round_half_even(1, 3), 33);
        assert_eq!(percent_round_half_even(2, 3), 67);
        assert_eq!(percent_round_half_even(1, 0), 0);
    }

    #[test]
    fn percent_matches_float_reference() {
        for total in 1..=400i64 {
            for good in 0..=total {
                let want = (100.0 * good as f64 / total as f64).round_ties_even() as i64;
                assert_eq!(percent_round_half_even(good, total), want, "{good}/{total}");
            }
        }
    }

    #[test]
    fn import_flow_added_updated_restored_removed() {
        let (_d, s) = open_tmp();
        let sh = vec![sheet("S", 3)];
        let qs = vec![
            question("S", "a", "A?", 1),
            question("S", "b", "B?", 2),
            question("S", "c", "C?", 3),
        ];
        let plan = s.plan_import(&sh, &qs).expect("plan");
        assert_eq!(plan["added"], 3);
        assert_eq!(
            s.count_active().expect("count"),
            0,
            "preview must not write"
        );
        let r = s.apply_import("one.xlsx", &sh, &qs).expect("apply");
        assert_eq!(r, plan);
        assert_eq!(s.count_active().expect("count"), 3);

        // b 改题干、c 消失、d 新增
        let qs2 = vec![
            question("S", "a", "A?", 5),
            question("S", "b", "B2?", 2),
            question("S", "d", "D?", 3),
        ];
        let r2 = s
            .apply_import("two.xlsx", &[sheet("S", 3)], &qs2)
            .expect("apply2");
        assert_eq!(
            (
                r2["added"].as_i64(),
                r2["updated"].as_i64(),
                r2["unchanged"].as_i64(),
                r2["removed"].as_i64()
            ),
            (Some(1), Some(1), Some(1), Some(1))
        );
        assert_eq!(r2["removed_samples"], json!(["C?"]));
        let bank = s.bank().expect("bank");
        assert_eq!(bank["file"], "two.xlsx");
        assert_eq!(bank["mtime"], "2026-01-02 03:04");
        assert_eq!(bank["sheets"][0]["inactive"], 1);
        assert_eq!(bank["questions"].as_array().map(Vec::len), Some(3));

        // 回到第一版：c 恢复
        let r3 = s.apply_import("three.xlsx", &sh, &qs).expect("apply3");
        assert_eq!(r3["restored"], 1);
        assert_eq!(s.import_history(10).expect("hist").len(), 3);
    }

    #[test]
    fn removed_sheet_only_in_database_is_appended() {
        let (_d, s) = open_tmp();
        s.apply_import(
            "a.xlsx",
            &[sheet("Old", 1)],
            &[question("Old", "o", "O?", 1)],
        )
        .expect("a");
        let plan = s
            .plan_import(&[sheet("New", 1)], &[question("New", "n", "N?", 1)])
            .expect("plan");
        let names: Vec<&str> = plan["sheets"]
            .as_array()
            .expect("arr")
            .iter()
            .filter_map(|x| x["name"].as_str())
            .collect();
        assert_eq!(names, vec!["New", "Old"]);
        assert_eq!(plan["sheets"][1]["removed"], 1);
        assert_eq!(plan["sheets"][1]["total"], 0);
    }

    #[test]
    fn attempt_updates_stats_and_prunes() {
        let (_d, s) = open_tmp();
        let mk = |ok: bool| Attempt {
            results: vec![
                AttemptResult {
                    id: "q1".to_string(),
                    correct: ok,
                },
                AttemptResult {
                    id: "q2".to_string(),
                    correct: true,
                },
            ],
            sheets: vec!["日本語".to_string()],
            mode: "all".to_string(),
            seconds: 7,
        };
        let p = s.record_attempt(&mk(false)).expect("rec");
        assert_eq!(p["questions"]["q1"]["wrong"], 1);
        assert_eq!(p["questions"]["q1"]["last"], false);
        assert_eq!(p["history"][0]["percent"], 50);
        for _ in 0..70 {
            s.record_attempt(&mk(true)).expect("rec");
        }
        let p = s.progress().expect("progress");
        assert_eq!(p["history"].as_array().map(Vec::len), Some(60));
        assert_eq!(p["questions"]["q1"]["attempts"], 71);
        let p = s.reset_progress().expect("reset");
        assert_eq!(p["questions"], json!({}));
        assert_eq!(p["history"], json!([]));
        assert!(s
            .record_attempt(&Attempt {
                results: vec![],
                sheets: vec![],
                mode: "all".into(),
                seconds: 0
            })
            .is_err());
    }

    #[test]
    fn leftover_transaction_is_rolled_back_when_lock_recovers() {
        let (_d, s) = open_tmp();
        {
            let g = s.lock();
            g.execute_batch("BEGIN IMMEDIATE").expect("begin");
            g.execute("INSERT INTO meta(key,value) VALUES('zzz','1')", [])
                .expect("insert");
        }
        // 模拟前一个调用在事务中途 panic：下一次取锁必须能恢复
        assert_eq!(get_meta(&s.lock(), "zzz").expect("meta"), None);
        s.reset_progress().expect("reset after recovery");
    }
}
