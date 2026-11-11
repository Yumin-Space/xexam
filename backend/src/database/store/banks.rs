//! Question banks (v0.5): one bank = one imported workbook.
//!
//! - The legacy bank (`legacy_ids=1`) holds the data from before v0.5 and is the target of uploads without `X-Bank-Target`; its
//!   question ids are the parser ids, unchanged, so old stats / wrong notebook / history stay linked.
//! - Every other bank uses bank-scoped ids ([`scoped_id`]), so the same workbook can be imported as two independent banks.
//! - `GET /api/banks` ([`Store::banks`]) and `POST /api/banks/delete` ([`Store::delete_bank`]).
//!
//! v0.6.5 (schema v4): who may see and manage a bank.
//! - Every bank has an owner (`banks.owner_id`): the user who imported it as a new bank (the first administrator for banks from
//!   before v0.6.5 and for banks created without a signed-in user, such as by the `import` command).
//! - [`viewer_of`] builds the [`Viewer`] of a request: administrators see and manage every bank; a plain user sees the banks
//!   listed in `user_banks`, plus, while they hold the "can manage question banks" permission, the banks they own; a bank
//!   manager may update and delete only the banks they own.
//! - v0.7: "include banks imported later" (`users.new_banks`): whenever a bank is created ([`insert_bank`], every path that
//!   inserts a `banks` row), the same transaction adds a `user_banks` row for every user with the flag on. Existing banks are
//!   never added by the flag, and unticking one bank removes only that bank's row.
//! - The `*_as` methods take that viewer: the bank list is limited to the visible banks, and deleting a bank checks the owner
//!   inside the same transaction as the delete ([`DbError::NotBankOwner`] = nothing written).
use super::records::snapshot_ids;
use super::{run_tx, users, Store, IN_CHUNK};
use crate::error::DbError;
use crate::models::{BankScope, DeletedBank, Question, Viewer};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

/// Name of a legacy bank whose workbook name is unknown (no import row).
pub const FALLBACK_BANK_NAME: &str = "Question bank";

/// A row of `banks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BankRow {
    pub id: i64,
    pub name: String,
    pub filename: String,
    pub legacy: bool,
    pub created_at: String,
    pub updated_at: String,
}

const BANK_COLUMNS: &str = "id, name, filename, legacy_ids, created_at, updated_at";

fn bank_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<BankRow> {
    Ok(BankRow {
        id: r.get(0)?,
        name: r.get(1)?,
        filename: r.get(2)?,
        legacy: r.get::<_, i64>(3)? != 0,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

/// Question id inside a non-legacy bank: the first 12 hex digits of `sha1("xexam-bank:{bank_id}\n{parser_id}")`.
pub fn scoped_id(bank_id: i64, parser_id: &str) -> String {
    let digest = Sha1::digest(format!("xexam-bank:{bank_id}\n{parser_id}").as_bytes());
    let mut out = String::with_capacity(12);
    for b in digest.iter().take(6) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// The questions with the ids they have in `bank`: unchanged for the legacy bank and for a bank that does not exist yet
/// (nothing to compare against), bank-scoped otherwise.
pub(super) fn scoped_questions<'a>(
    bank: Option<&BankRow>,
    questions: &'a [Question],
) -> Cow<'a, [Question]> {
    match bank {
        Some(b) if !b.legacy => Cow::Owned(
            questions
                .iter()
                .map(|q| {
                    let mut q = q.clone();
                    q.id = scoped_id(b.id, &q.id);
                    q
                })
                .collect(),
        ),
        _ => Cow::Borrowed(questions),
    }
}

pub(super) fn legacy_bank(c: &Connection) -> Result<Option<BankRow>, DbError> {
    Ok(c.query_row(
        &format!("SELECT {BANK_COLUMNS} FROM banks WHERE legacy_ids=1 ORDER BY id LIMIT 1"),
        [],
        bank_row,
    )
    .optional()?)
}

pub(super) fn bank_by_id(c: &Connection, id: i64) -> Result<Option<BankRow>, DbError> {
    Ok(c.query_row(
        &format!("SELECT {BANK_COLUMNS} FROM banks WHERE id=?1"),
        params![id],
        bank_row,
    )
    .optional()?)
}

/// The upload file name without a trailing `.xlsx` (case-insensitive); the whole name if that leaves nothing.
pub(super) fn name_from_filename(filename: &str) -> String {
    let n = filename.len();
    let stem = match filename.get(n.saturating_sub(5)..) {
        Some(ext) if n >= 5 && ext.eq_ignore_ascii_case(".xlsx") => filename.get(..n - 5),
        _ => None,
    };
    match stem {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => filename.to_string(),
    }
}

/// `base`, or `base (2)`, `base (3)`, … — the first one no other bank uses.
fn unique_name(c: &Connection, base: &str) -> Result<String, DbError> {
    let taken: HashSet<String> = {
        let mut st = c.prepare("SELECT name FROM banks")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut name = base.to_string();
    let mut n = 2u64;
    while taken.contains(&name) {
        name = format!("{base} ({n})");
        n += 1;
    }
    Ok(name)
}

/// Inserts a bank row (inside the caller's transaction). Every bank this version creates goes through here: the users who
/// chose "include banks imported later" get it at once ([`bank_created`]).
pub(super) fn insert_bank(
    c: &Connection,
    name: &str,
    filename: &str,
    legacy: bool,
    created_at: &str,
    updated_at: &str,
) -> Result<i64, DbError> {
    c.execute(
        "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at) VALUES(?1,?2,?3,?4,?5)",
        params![name, filename, i64::from(legacy), created_at, updated_at],
    )?;
    let id = c.last_insert_rowid();
    bank_created(c, id)?;
    Ok(id)
}

/// The `meta` key holding the highest bank id that this version (v0.7 or later) knows about. A bank with a higher id was
/// created by an older binary after a rollback, which does not add it for the users with "include banks imported later":
/// the fix-up on open does (see `migrate.rs`).
pub(super) const KNOWN_BANKS_KEY: &str = "banks_known";

/// Records the highest bank id as known to this version (inside the caller's transaction). Bank ids are never reused
/// (`AUTOINCREMENT`), so every bank an older binary creates later has a higher id.
pub(super) fn note_known_banks(c: &Connection) -> Result<(), DbError> {
    c.execute(
        "INSERT INTO meta(key, value) SELECT ?1, CAST(COALESCE(MAX(id), 0) AS TEXT) FROM banks \
         WHERE 1 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![KNOWN_BANKS_KEY],
    )?;
    Ok(())
}

/// A bank was just created (inside the same transaction): every user with "include banks imported later" (`new_banks = 1`)
/// gets a `user_banks` row for it, and the bank is known to this version. Nothing happens while the users table does not
/// exist yet (the v1 → v2 step of a migration creates the legacy bank before the v3 step adds users; the v4 step then gives
/// every user every bank).
fn bank_created(c: &Connection, id: i64) -> Result<(), DbError> {
    let has_flag: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('users') WHERE name='new_banks')",
        [],
        |r| r.get(0),
    )?;
    if !has_flag {
        return Ok(());
    }
    c.execute(
        "INSERT OR IGNORE INTO user_banks(user_id, bank_id) SELECT id, ?1 FROM users WHERE new_banks <> 0",
        params![id],
    )?;
    note_known_banks(c)
}

/// Creates a bank for an upload named `filename` (inside the caller's transaction), named after the file and owned by `owner`
/// (`None`: no signed-in user, e.g. the `import` command: the first administrator owns it).
pub(super) fn create_bank(
    c: &Connection,
    filename: &str,
    legacy: bool,
    now: &str,
    owner: Option<i64>,
) -> Result<BankRow, DbError> {
    let name = unique_name(c, &name_from_filename(filename))?;
    let id = insert_bank(c, &name, filename, legacy, now, now)?;
    // `insert_bank` also serves the v2 migration, which runs before `owner_id` exists, so the owner is set here
    let owner = match owner {
        Some(o) => Some(o),
        None => users::owner_admin(c)?,
    };
    c.execute(
        "UPDATE banks SET owner_id=?1 WHERE id=?2",
        params![owner, id],
    )?;
    Ok(BankRow {
        id,
        name,
        filename: filename.to_string(),
        legacy,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    })
}

/// The owner of bank `id` (`None`: no such bank, or a bank without a recorded owner).
pub(super) fn bank_owner(c: &Connection, id: i64) -> Result<Option<i64>, DbError> {
    Ok(
        c.query_row("SELECT owner_id FROM banks WHERE id=?1", params![id], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .optional()?
        .flatten(),
    )
}

/// The ids of the banks `user` owns.
pub(super) fn owned_banks(c: &Connection, user: i64) -> Result<BTreeSet<i64>, DbError> {
    let mut st = c.prepare("SELECT id FROM banks WHERE owner_id=?1")?;
    let rows = st.query_map(params![user], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The [`Viewer`] of the signed-in user `user`, read from the database now (so a change made by an administrator applies to
/// the very next request). A user that no longer exists sees nothing and manages nothing.
pub(super) fn viewer_of(c: &Connection, user: i64) -> Result<Viewer, DbError> {
    let row: Option<(String, bool)> = c
        .query_row(
            "SELECT role, can_manage_banks FROM users WHERE id=?1",
            params![user],
            |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
        )
        .optional()?;
    let Some((role, can_manage)) = row else {
        return Ok(Viewer {
            user,
            admin: false,
            can_manage: false,
            scope: BankScope::Only(BTreeSet::new()),
        });
    };
    if role == crate::auth::Role::Admin.as_str() {
        return Ok(Viewer::admin(user));
    }
    // A plain user sees exactly their `user_banks` rows ("include banks imported later" only ever adds rows)
    let mut ids: BTreeSet<i64> = {
        // Only banks that still exist (a row left behind by an older binary must not grant a later bank with that id)
        let mut st = c.prepare(
            "SELECT bank_id FROM user_banks WHERE user_id=?1 AND bank_id IN (SELECT id FROM banks)",
        )?;
        let rows = st.query_map(params![user], |r| r.get::<_, i64>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    // A bank manager always sees the banks they own (they imported them to study them)
    if can_manage {
        ids.extend(owned_banks(c, user)?);
    }
    let scope = BankScope::Only(ids);
    Ok(Viewer {
        user,
        admin: false,
        can_manage,
        scope,
    })
}

#[derive(Default)]
struct BankAgg {
    total: i64,
    inactive: i64,
    /// (sheet, usable, inactive) in order of first appearance
    sheets: Vec<(String, i64, i64)>,
}

/// `{"banks": [...]}` for `GET /api/banks` as `viewer` sees it, ordered by id: only the banks they may see, each with its owner
/// and whether the viewer may update / delete it. Administrators get the shared numbers (`studied` / `records` count every
/// user's data, which their deletion with records removes); everyone else gets their own numbers (a bank manager's deletion
/// with records removes only their own).
fn banks_json(c: &Connection, viewer: &Viewer) -> Result<Value, DbError> {
    let banks: Vec<BankRow> = {
        let mut st = c.prepare(&format!("SELECT {BANK_COLUMNS} FROM banks ORDER BY id"))?;
        let rows = st.query_map([], bank_row)?;
        rows.collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|b| viewer.scope.allows(Some(b.id)))
            .collect()
    };
    let owners: HashMap<i64, Option<i64>> = {
        let mut st = c.prepare("SELECT id, owner_id FROM banks")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let usernames: HashMap<i64, String> = {
        let mut st = c.prepare("SELECT id, username FROM users")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    // Same query shape and row order as `/api/bank` (a full scan sorted by `seq`), so the sheet order matches it
    let mut aggs: HashMap<i64, BankAgg> = HashMap::new();
    {
        let mut st = c.prepare("SELECT bank_id, sheet, active FROM questions ORDER BY seq")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let Some(bank_id) = r.get::<_, Option<i64>>(0)? else {
                continue;
            };
            let sheet: String = r.get(1)?;
            let active = r.get::<_, i64>(2)? != 0;
            let agg = aggs.entry(bank_id).or_default();
            let i = match agg.sheets.iter().position(|(name, _, _)| *name == sheet) {
                Some(i) => i,
                None => {
                    agg.sheets.push((sheet, 0, 0));
                    agg.sheets.len() - 1
                }
            };
            if let Some(entry) = agg.sheets.get_mut(i) {
                if active {
                    entry.1 += 1;
                } else {
                    entry.2 += 1;
                }
            }
            if active {
                agg.total += 1;
            } else {
                agg.inactive += 1;
            }
        }
    }
    let studied: HashMap<i64, i64> = if viewer.admin {
        // Shared numbers: a question counts once if any user has statistics for it
        let mut st = c.prepare(
            "SELECT q.bank_id, COUNT(*) FROM (SELECT DISTINCT question_id FROM user_stats) s JOIN questions q ON q.id = s.question_id \
             WHERE q.bank_id IS NOT NULL GROUP BY q.bank_id",
        )?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    } else {
        let mut st = c.prepare(
            "SELECT q.bank_id, COUNT(*) FROM user_stats s JOIN questions q ON q.id = s.question_id \
             WHERE s.user_id=?1 AND q.bank_id IS NOT NULL GROUP BY q.bank_id",
        )?;
        let rows = st.query_map(params![viewer.user], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut records: HashMap<i64, i64> = HashMap::new();
    {
        // Administrators: every user's records (`?1` is NULL); everyone else: their own
        let user = (!viewer.admin).then_some(viewer.user);
        let mut st = c.prepare("SELECT banks FROM attempts WHERE ?1 IS NULL OR user_id=?1")?;
        let mut rows = st.query(params![user])?;
        while let Some(r) = rows.next()? {
            let ids: HashSet<i64> = snapshot_ids(&r.get::<_, String>(0)?).into_iter().collect();
            for id in ids {
                *records.entry(id).or_default() += 1;
            }
        }
    }
    let list: Vec<Value> = banks
        .iter()
        .map(|b| {
            let owner = owners.get(&b.id).copied().flatten();
            let agg = aggs.remove(&b.id).unwrap_or_default();
            let sheets: Vec<Value> = agg
                .sheets
                .iter()
                .map(|(name, usable, inactive)| json!({"name": name, "usable": usable, "inactive": inactive}))
                .collect();
            json!({
                "id": b.id,
                "name": b.name,
                "filename": b.filename,
                "legacy": b.legacy,
                "createdAt": b.created_at,
                "updatedAt": b.updated_at,
                "total": agg.total,
                "inactive": agg.inactive,
                "sheets": sheets,
                "studied": studied.get(&b.id).copied().unwrap_or(0),
                "records": records.get(&b.id).copied().unwrap_or(0),
                // Only administrators learn who owns a bank (the user list is theirs alone); anyone else is told only
                // whether it is their own bank, so the bank list never names other accounts
                "ownerId": if viewer.admin { owner } else { owner.filter(|o| *o == viewer.user) },
                "owner": if viewer.admin { owner.and_then(|o| usernames.get(&o)) } else { None },
                "manageable": viewer.manages(owner),
            })
        })
        .collect();
    Ok(json!({ "banks": list }))
}

/// Runs `sql` (with one `{marks}` placeholder for `?,?,…`) for every chunk of `ids`.
fn exec_in_chunks(c: &Connection, sql: &str, ids: &[String]) -> Result<(), DbError> {
    for chunk in ids.chunks(IN_CHUNK) {
        let marks = vec!["?"; chunk.len()].join(",");
        c.execute(
            &sql.replace("{marks}", &marks),
            rusqlite::params_from_iter(chunk.iter()),
        )?;
    }
    Ok(())
}

impl Store {
    /// The number of question banks (active questions or not).
    pub fn bank_count(&self) -> Result<i64, DbError> {
        let g = self.lock();
        Ok(g.query_row("SELECT COUNT(*) FROM banks", [], |r| r.get(0))?)
    }

    pub fn bank_exists(&self, id: i64) -> Result<bool, DbError> {
        let g = self.lock();
        Ok(bank_by_id(&g, id)?.is_some())
    }

    /// `GET /api/banks` as an administrator sees it: `{"banks": [{"id", "name", "filename", "legacy", "createdAt",
    /// "updatedAt", "total", "inactive", "sheets": [{"name", "usable", "inactive"}], "studied", "records", "ownerId", "owner",
    /// "manageable"}]}` (`ownerId` / `owner`: the owner's id and username, null when unknown).
    pub fn banks(&self) -> Result<Value, DbError> {
        let g = self.lock();
        banks_json(&g, &Viewer::admin(0))
    }

    /// `GET /api/banks` as `viewer` sees it (see [`banks_json`]): only their visible banks; `manageable` says whether they may
    /// update and delete it.
    pub fn banks_as(&self, viewer: &Viewer) -> Result<Value, DbError> {
        let g = self.lock();
        banks_json(&g, viewer)
    }

    /// The [`Viewer`] of the signed-in user `user` (see [`viewer_of`]).
    pub fn viewer(&self, user: i64) -> Result<Viewer, DbError> {
        let g = self.lock();
        viewer_of(&g, user)
    }

    /// Whether `viewer` may write an import into `target`: `Ok(true)` yes, `Ok(false)` the bank does not exist,
    /// [`DbError::NotBankOwner`] it exists but they may not update it. A new bank is always allowed (the route already checked
    /// the permission); so is the legacy target while there is no legacy bank (it would be created, owned by them).
    pub fn check_import_target(
        &self,
        viewer: &Viewer,
        target: crate::models::ImportTarget,
    ) -> Result<bool, DbError> {
        use crate::models::ImportTarget;
        let g = self.lock();
        let bank = match target {
            ImportTarget::New => return Ok(true),
            ImportTarget::Legacy => match legacy_bank(&g)? {
                Some(b) => b,
                None => return Ok(true),
            },
            ImportTarget::Bank(id) => match bank_by_id(&g, id)? {
                Some(b) => b,
                None => return Ok(false),
            },
        };
        if viewer.manages(bank_owner(&g, bank.id)?) {
            Ok(true)
        } else {
            Err(DbError::NotBankOwner)
        }
    }

    /// An opaque stamp of the whole question library as an administrator sees it (`"library"` of `GET /api/auth/state`): it
    /// changes whenever a bank is imported, replaced or deleted, also by another process (the `import` command). Cheap: a few
    /// aggregates, no question text. It is a digest, so it does not publish the counters it is made of.
    pub fn library_stamp(&self) -> Result<String, DbError> {
        let g = self.lock();
        Ok(stamp_digest(&library_stamp_of(&g)?))
    }

    /// [`Store::library_stamp`] for one viewer: it changes when one of the banks they may see changes, and when the banks they
    /// may see or manage change (an administrator assigned banks, gave or took the permission, a bank changed owner), so their
    /// open pages re-read the banks. A viewer limited to some banks gets a stamp made only of those banks: it neither reveals
    /// nor follows the size, the question count or the update times of the banks they cannot see.
    pub fn library_stamp_as(&self, viewer: &Viewer) -> Result<String, DbError> {
        let g = self.lock();
        // Administrators see and manage every bank (and a viewer with the scope of everything who manages none sees exactly
        // what they do): their stamp is the shared one. Anyone else's also depends on what they may see and which banks they
        // own. Since v0.7 a plain user's scope is always their own list of banks.
        if viewer.admin || (viewer.scope.is_all() && !viewer.can_manage) {
            return Ok(stamp_digest(&library_stamp_of(&g)?));
        }
        let seen = match &viewer.scope {
            BankScope::All => library_stamp_of(&g)?,
            BankScope::Only(ids) => visible_banks_stamp(&g, ids)?,
        };
        let owned = owned_banks(&g, viewer.user)?;
        Ok(stamp_digest(&format!(
            "{seen}|{}|{:?}|{:?}",
            viewer.can_manage, viewer.scope, owned
        )))
    }
}

/// The library stamp as sent: the first 16 bytes of the SHA-1 of `raw`, in hex.
fn stamp_digest(raw: &str) -> String {
    let digest = Sha1::digest(raw.as_bytes());
    let mut out = String::with_capacity(32);
    for b in digest.iter().take(16) {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// What the stamp of the whole library is made of (see [`Store::library_stamp`]): the number of banks, the highest bank and
/// import ids, the number of active questions and the last update time.
fn library_stamp_of(g: &Connection) -> Result<String, DbError> {
    let (banks, last_bank, updated): (i64, i64, String) = g.query_row(
        "SELECT COUNT(*), COALESCE(MAX(id), 0), COALESCE(MAX(updated_at), '') FROM banks",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let last_import: i64 =
        g.query_row("SELECT COALESCE(MAX(id), 0) FROM imports", [], |r| r.get(0))?;
    let active: i64 = g.query_row("SELECT COUNT(*) FROM questions WHERE active=1", [], |r| {
        r.get(0)
    })?;
    Ok(format!(
        "{banks}.{last_bank}.{last_import}.{active}.{}",
        updated.replace(['-', ':', ' '], "")
    ))
}

/// What the stamp of a viewer limited to the banks `ids` is made of: for each of those banks (ascending ids) its update time,
/// its number of active questions and its latest import id (`-` for a bank that no longer exists). Nothing about other banks.
fn visible_banks_stamp(g: &Connection, ids: &BTreeSet<i64>) -> Result<String, DbError> {
    let mut st = g.prepare(
        "SELECT b.updated_at, \
         (SELECT COUNT(*) FROM questions q WHERE q.bank_id=b.id AND q.active=1), \
         (SELECT COALESCE(MAX(i.id), 0) FROM imports i WHERE i.bank_id=b.id) \
         FROM banks b WHERE b.id=?1",
    )?;
    let mut out = String::new();
    for id in ids {
        let row: Option<(String, i64, i64)> = st
            .query_row(params![id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        match row {
            Some((updated, active, import)) => {
                let _ = write!(out, "{id}:{updated}:{active}:{import};");
            }
            None => {
                let _ = write!(out, "{id}:-;");
            }
        }
    }
    Ok(out)
}

impl Store {
    /// Deletes bank `id` as an administrator (see [`Store::delete_bank_as`]): with `with_records`, the study data of every user.
    pub fn delete_bank(&self, id: i64, with_records: bool) -> Result<Option<DeletedBank>, DbError> {
        self.delete_bank_as(&Viewer::admin(0), id, with_records)
    }

    /// Deletes bank `id` in one transaction: its questions (active and retired), its import history, who may see it
    /// (`user_banks`) and the bank row. With `with_records` also its study data: the `user_stats` and `attempt_items` rows of
    /// those questions, and every exam record whose `banks` snapshot names only this bank or banks deleted earlier (with the
    /// rest of its detail) — of every user when an administrator deletes it, of the viewer only otherwise (other users'
    /// records stay; their questions then show as deleted). `Ok(None)` = no such bank; [`DbError::NotBankOwner`] = the viewer
    /// may not delete it (both: nothing written).
    pub fn delete_bank_as(
        &self,
        viewer: &Viewer,
        id: i64,
        with_records: bool,
    ) -> Result<Option<DeletedBank>, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            let Some(bank) = bank_by_id(c, id)? else {
                return Ok(None);
            };
            // Checked inside the transaction: an owner change (a deleted user's banks move to an administrator) cannot slip in
            if !viewer.manages(bank_owner(c, id)?) {
                return Err(DbError::NotBankOwner);
            }
            // `?N IS NULL OR user_id=?N`: every user (administrator) or only the viewer
            let only_user = (!viewer.admin).then_some(viewer.user);
            let question_ids: Vec<String> = {
                let mut st = c.prepare("SELECT id FROM questions WHERE bank_id=?1")?;
                let rows = st.query_map(params![id], |r| r.get::<_, String>(0))?;
                rows.collect::<Result<_, _>>()?
            };
            c.execute("DELETE FROM questions WHERE bank_id=?1", params![id])?;
            c.execute("DELETE FROM imports WHERE bank_id=?1", params![id])?;
            c.execute("DELETE FROM user_banks WHERE bank_id=?1", params![id])?;
            c.execute("DELETE FROM banks WHERE id=?1", params![id])?;
            if with_records {
                match only_user {
                    None => {
                        exec_in_chunks(
                            c,
                            "DELETE FROM user_stats WHERE question_id IN ({marks})",
                            &question_ids,
                        )?;
                        exec_in_chunks(
                            c,
                            "DELETE FROM attempt_items WHERE question_id IN ({marks})",
                            &question_ids,
                        )?;
                    }
                    Some(user) => {
                        let user_sql = user.to_string();
                        exec_in_chunks(
                            c,
                            &format!(
                                "DELETE FROM user_stats WHERE user_id={user_sql} AND question_id IN ({{marks}})"
                            ),
                            &question_ids,
                        )?;
                        exec_in_chunks(
                            c,
                            &format!(
                                "DELETE FROM attempt_items WHERE question_id IN ({{marks}}) \
                                 AND attempt_id IN (SELECT id FROM attempts WHERE user_id={user_sql})"
                            ),
                            &question_ids,
                        )?;
                    }
                }
                // Exams that drew from this bank and from no bank that still exists (an exam mixing this bank with one
                // deleted earlier would otherwise be left behind as an empty record)
                let remaining: HashSet<i64> = {
                    let mut st = c.prepare("SELECT id FROM banks")?;
                    let rows = st.query_map([], |r| r.get::<_, i64>(0))?;
                    rows.collect::<Result<_, _>>()?
                };
                let only_this: Vec<i64> = {
                    let mut st =
                        c.prepare("SELECT id, banks FROM attempts WHERE ?1 IS NULL OR user_id=?1")?;
                    let rows = st.query_map(params![only_user], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })?;
                    let mut out = Vec::new();
                    for row in rows {
                        let (attempt_id, banks) = row?;
                        let ids = snapshot_ids(&banks);
                        if ids.contains(&id) && ids.iter().all(|b| !remaining.contains(b)) {
                            out.push(attempt_id);
                        }
                    }
                    out
                };
                for attempt_id in only_this {
                    c.execute(
                        "DELETE FROM attempt_items WHERE attempt_id=?1",
                        params![attempt_id],
                    )?;
                    c.execute("DELETE FROM attempts WHERE id=?1", params![attempt_id])?;
                }
            }
            Ok(Some(DeletedBank {
                id,
                legacy: bank.legacy,
                name: bank.name,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_ids_are_twelve_hex_and_depend_on_the_bank() {
        let a = scoped_id(2, "f3886e9f5948");
        assert_eq!(a.len(), 12);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, scoped_id(3, "f3886e9f5948"));
        assert_ne!(a, scoped_id(2, "f3886e9f5949"));
        assert_eq!(a, scoped_id(2, "f3886e9f5948"));
        // the documented formula, computed independently
        let digest = Sha1::digest(b"xexam-bank:2\nf3886e9f5948");
        let want: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
        assert_eq!(a, want);
    }

    #[test]
    fn names_from_file_names() {
        assert_eq!(name_from_filename("Kitchen basics.xlsx"), "Kitchen basics");
        assert_eq!(name_from_filename("Kitchen basics.XLSX"), "Kitchen basics");
        assert_eq!(name_from_filename("a.b.Xlsx"), "a.b");
        assert_eq!(name_from_filename(".xlsx"), ".xlsx");
        assert_eq!(name_from_filename("notes.xls"), "notes.xls");
        assert_eq!(name_from_filename("日本語.xlsx"), "日本語");
        assert_eq!(name_from_filename("日.xlsx"), "日");
        assert_eq!(name_from_filename("xlsx"), "xlsx");
        assert_eq!(name_from_filename(""), "");
    }
}
