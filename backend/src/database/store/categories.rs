//! v3.1 bank categories (contracts A.2, A.7, B.10, D18): one level, edited by administrators only.
//!
//! - `bank_categories` holds the categories; `banks.category_id` (NULL = uncategorised) and `banks.sort` place a bank inside its
//!   group. Groups are ordered by `((sort = 0), sort, id)`: `sort = 0` means "not placed yet" (every bank created by any path),
//!   so unplaced banks follow the placed ones in id order and a new bank lands at the end of its group.
//! - Every write here is one transaction. Moving banks never touches `banks.updated_at` (it is content time, part of the
//!   library stamp), so category edits never change the `/api/bank` ETag.
//! - The delete order is child-first (A.7): banks leave the category before its row goes.
//! - A refusal ([`CategoryError`]) is always decided before the first write of its operation, so the transaction that carries
//!   it back out commits nothing.

use super::{run_tx, Store};
use crate::error::DbError;
use crate::models::Viewer;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::fmt::Write as _;

// The SQL texts of the E.1 split live in `banks`; re-exported so the query-plan test can run the very statements.
pub use super::banks::{
    ATTEMPT_BANKS_ALL_SQL, ATTEMPT_BANKS_USER_SQL, ATTEMPT_IDS_ALL_SQL, ATTEMPT_IDS_USER_SQL,
    ATTEMPT_STAMP_ALL_SQL, ATTEMPT_STAMP_USER_SQL, STATS_STAMP_ALL_SQL, STATS_STAMP_USER_SQL,
};

/// The longest category name, in characters.
pub const CATEGORY_NAME_MAX_CHARS: usize = 60;

/// Why a category operation did not happen (nothing was written in any of these cases).
#[derive(Debug)]
pub enum CategoryError {
    /// The name is empty after cleaning, too long, or holds a control / invisible format character (400 `banks.categoryName`)
    NameBad,
    /// Another category already has this name, compared without regard to ASCII case (409 `banks.categoryTaken`)
    Taken,
    /// The category does not exist (404 `banks.categoryNotFound`)
    NotFound,
    /// The ids are not exactly the set the operation needs (409 `banks.categoryOrder`)
    Order,
    /// A bank id does not exist (404 `MSG_BANK_NOT_FOUND`)
    BankNotFound,
    /// The request itself is malformed (400), for example the same id twice in a move
    BadRequest,
    Db(DbError),
}

impl From<DbError> for CategoryError {
    fn from(e: DbError) -> Self {
        CategoryError::Db(e)
    }
}

/// One operation of `POST /api/banks/categories`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryOp {
    Create {
        name: String,
    },
    Rename {
        id: i64,
        name: String,
    },
    Delete {
        id: i64,
    },
    /// Every category id, in the new order
    Order {
        ids: Vec<i64>,
    },
    /// Exactly the banks of the group (`None` = uncategorised), in the new order
    BankOrder {
        category_id: Option<i64>,
        ids: Vec<i64>,
    },
}

/// Invisible format characters (zero-width, bidi controls, soft hyphen, BOM): refused so two names cannot look the same.
fn is_invisible_format(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}')
}

/// The category name rule: whitespace runs (tabs and line breaks too) become one space, the ends are trimmed, 1..=60 characters
/// remain and none is a control or invisible format character. Returns the cleaned name.
pub fn clean_category_name(raw: &str) -> Result<String, CategoryError> {
    // Control characters other than whitespace are refused before the whitespace collapses
    if raw
        .chars()
        .any(|c| (c.is_control() && !c.is_whitespace()) || is_invisible_format(c))
    {
        return Err(CategoryError::NameBad);
    }
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let n = name.chars().count();
    if (1..=CATEGORY_NAME_MAX_CHARS).contains(&n) {
        Ok(name)
    } else {
        Err(CategoryError::NameBad)
    }
}

/// The ids of the banks of one group in their display order `((sort = 0), sort, id)`; `None` = uncategorised.
fn group_ids(c: &Connection, category: Option<i64>) -> Result<Vec<i64>, DbError> {
    let mut st =
        c.prepare("SELECT id FROM banks WHERE category_id IS ?1 ORDER BY (sort = 0), sort, id")?;
    let rows = st.query_map(params![category], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn category_exists(c: &Connection, id: i64) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT 1 FROM bank_categories WHERE id=?1",
        params![id],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}

/// Every category id in `(sort, id)` order.
fn category_ids(c: &Connection) -> Result<Vec<i64>, DbError> {
    let mut st = c.prepare("SELECT id FROM bank_categories ORDER BY sort, id")?;
    let rows = st.query_map([], |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Writes `category_id` and `sort = index + 1` for `ids` (a group, in order).
fn place_banks(c: &Connection, category: Option<i64>, ids: &[i64]) -> Result<(), DbError> {
    let mut st = c.prepare("UPDATE banks SET category_id=?1, sort=?2 WHERE id=?3")?;
    for (i, id) in ids.iter().enumerate() {
        st.execute(params![category, i as i64 + 1, id])?;
    }
    Ok(())
}

/// `ids` has no duplicate and holds exactly the members of `want`.
fn same_set(ids: &[i64], want: &[i64]) -> bool {
    let a: HashSet<i64> = ids.iter().copied().collect();
    a.len() == ids.len() && a.len() == want.len() && want.iter().all(|w| a.contains(w))
}

/// True when another category (not `except`) already has `name` (SQLite `NOCASE`: ASCII letters fold).
fn name_taken(c: &Connection, name: &str, except: Option<i64>) -> Result<bool, DbError> {
    Ok(c.query_row(
        "SELECT 1 FROM bank_categories WHERE name = ?1 COLLATE NOCASE AND id IS NOT ?2",
        params![name, except],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}

fn bank_visible(viewer: &Viewer, bank: i64) -> bool {
    viewer.admin || viewer.scope.allows(Some(bank))
}

/// The categories the viewer is shown, as `(id, name, sort)` in `(sort, id)` order: all of them for an administrator; everyone
/// else only those holding at least one bank they can see.
fn visible_categories(c: &Connection, viewer: &Viewer) -> Result<Vec<(i64, String, i64)>, DbError> {
    let mut st = c.prepare("SELECT id, name, sort FROM bank_categories ORDER BY sort, id")?;
    let all: Vec<(i64, String, i64)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    if viewer.admin {
        return Ok(all);
    }
    let mut st = c.prepare("SELECT id, category_id FROM banks WHERE category_id IS NOT NULL")?;
    let shown: HashSet<i64> = st
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|(bank, _)| bank_visible(viewer, *bank))
        .map(|(_, category)| category)
        .collect();
    Ok(all
        .into_iter()
        .filter(|(id, _, _)| shown.contains(id))
        .collect())
}

/// `"categories"` of `GET /api/banks`: `[{"id", "name", "sort"}]` as the viewer is shown them.
pub(super) fn visible_categories_json(c: &Connection, viewer: &Viewer) -> Result<Value, DbError> {
    Ok(Value::Array(
        visible_categories(c, viewer)?
            .into_iter()
            .map(|(id, name, sort)| json!({"id": id, "name": name, "sort": sort}))
            .collect(),
    ))
}

/// What the `GET /api/banks` ETag adds for categories: every category `id:name:sort` and every bank `id:category_id:sort`
/// (global, like the bank names the stamp already holds).
pub(super) fn placement_stamp(c: &Connection) -> Result<String, DbError> {
    let cats: String = c.query_row(
        "SELECT COALESCE(group_concat(id || ':' || name || ':' || sort, ';'), '') \
         FROM (SELECT id, name, sort FROM bank_categories ORDER BY id)",
        [],
        |r| r.get(0),
    )?;
    let banks: String = c.query_row(
        "SELECT COALESCE(group_concat(id || ':' || COALESCE(category_id, '') || ':' || sort, ';'), '') \
         FROM (SELECT id, category_id, sort FROM banks ORDER BY id)",
        [],
        |r| r.get(0),
    )?;
    Ok(format!("{cats}|{banks}"))
}

impl Store {
    /// The opaque stamp of the categories and the placement of the banks the viewer sees (`categories` of
    /// `GET /api/auth/state`): administrators every category and every bank's `id:category_id:sort`; everyone else the
    /// categories they are shown and their visible banks.
    pub fn categories_stamp_as(&self, v: &Viewer) -> Result<String, DbError> {
        let g = self.lock();
        let mut raw = String::new();
        for (id, name, sort) in visible_categories(&g, v)? {
            let _ = write!(raw, "{id}:{name}:{sort};");
        }
        raw.push('|');
        let mut st = g.prepare("SELECT id, category_id, sort FROM banks ORDER BY id")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (id, category, sort) = row?;
            if bank_visible(v, id) {
                let cat = category.map(|c| c.to_string()).unwrap_or_default();
                let _ = write!(raw, "{id}:{cat}:{sort};");
            }
        }
        let digest = Sha1::digest(raw.as_bytes());
        let mut out = String::with_capacity(32);
        for b in digest.iter().take(16) {
            let _ = write!(out, "{b:02x}");
        }
        Ok(out)
    }

    /// Runs one operation of `POST /api/banks/categories` in a single transaction. Returns the category name for the activity
    /// note (`Some` for create / rename / delete).
    pub fn category_apply(&self, op: &CategoryOp) -> Result<Option<String>, CategoryError> {
        // Clean the name before the lock is taken: a refused name costs nothing
        let cleaned = match op {
            CategoryOp::Create { name } | CategoryOp::Rename { name, .. } => {
                Some(clean_category_name(name)?)
            }
            _ => None,
        };
        let now = self.unix_now();
        let g = self.lock();
        run_tx(&g, |c| apply_op(c, op, cleaned.as_deref(), now))?
    }

    /// `POST /api/banks/category`: moves the banks `ids` (distinct, in the order given) to the end of the group `category`
    /// (`None` = uncategorised). The moved banks leave the target group's order first, the group is renumbered 1..n, the moved
    /// banks get the next positions, and the groups they left are renumbered 1..n. `banks.updated_at` is untouched.
    pub fn banks_move_to_category(
        &self,
        ids: &[i64],
        category: Option<i64>,
    ) -> Result<(), CategoryError> {
        let g = self.lock();
        run_tx(&g, |c| move_banks(c, ids, category))?
    }
}

/// Outcome of one operation inside its transaction: `Ok(Err(..))` is a refusal decided before any write.
type Outcome<T> = Result<Result<T, CategoryError>, DbError>;

fn apply_op(
    c: &Connection,
    op: &CategoryOp,
    name: Option<&str>,
    now: i64,
) -> Outcome<Option<String>> {
    match op {
        CategoryOp::Create { .. } => {
            let name = name.unwrap_or_default();
            if name_taken(c, name, None)? {
                return Ok(Err(CategoryError::Taken));
            }
            let sort: i64 = c.query_row(
                "SELECT COALESCE(MAX(sort) + 1, 0) FROM bank_categories",
                [],
                |r| r.get(0),
            )?;
            c.execute(
                "INSERT INTO bank_categories(name, sort, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                params![name, sort, now],
            )?;
            Ok(Ok(Some(name.to_string())))
        }
        CategoryOp::Rename { id, .. } => {
            let name = name.unwrap_or_default();
            if !category_exists(c, *id)? {
                return Ok(Err(CategoryError::NotFound));
            }
            if name_taken(c, name, Some(*id))? {
                return Ok(Err(CategoryError::Taken));
            }
            c.execute(
                "UPDATE bank_categories SET name=?1, updated_at=?2 WHERE id=?3",
                params![name, now, id],
            )?;
            Ok(Ok(Some(name.to_string())))
        }
        CategoryOp::Delete { id } => {
            let name: Option<String> = c
                .query_row(
                    "SELECT name FROM bank_categories WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(name) = name else {
                return Ok(Err(CategoryError::NotFound));
            };
            // The banks of the category go to the end of the uncategorised group; the whole group gets fixed places
            let mut order = group_ids(c, None)?;
            order.extend(group_ids(c, Some(*id))?);
            place_banks(c, None, &order)?;
            c.execute("DELETE FROM bank_categories WHERE id=?1", params![id])?;
            write_category_order(c, &category_ids(c)?)?;
            Ok(Ok(Some(name)))
        }
        CategoryOp::Order { ids } => {
            if !same_set(ids, &category_ids(c)?) {
                return Ok(Err(CategoryError::Order));
            }
            write_category_order(c, ids)?;
            Ok(Ok(None))
        }
        CategoryOp::BankOrder { category_id, ids } => {
            if let Some(cat) = category_id {
                if !category_exists(c, *cat)? {
                    return Ok(Err(CategoryError::NotFound));
                }
            }
            if !same_set(ids, &group_ids(c, *category_id)?) {
                return Ok(Err(CategoryError::Order));
            }
            place_banks(c, *category_id, ids)?;
            Ok(Ok(None))
        }
    }
}

/// Writes `sort = index` (0..n-1) for the categories `ids`.
fn write_category_order(c: &Connection, ids: &[i64]) -> Result<(), DbError> {
    let mut st = c.prepare("UPDATE bank_categories SET sort=?1 WHERE id=?2")?;
    for (i, id) in ids.iter().enumerate() {
        st.execute(params![i as i64, id])?;
    }
    Ok(())
}

fn move_banks(c: &Connection, ids: &[i64], category: Option<i64>) -> Outcome<()> {
    let moved: HashSet<i64> = ids.iter().copied().collect();
    if ids.is_empty() || moved.len() != ids.len() {
        return Ok(Err(CategoryError::BadRequest));
    }
    if let Some(cat) = category {
        if !category_exists(c, cat)? {
            return Ok(Err(CategoryError::NotFound));
        }
    }
    // Where each moved bank sits now (this also proves that it exists)
    let mut sources: Vec<Option<i64>> = Vec::new();
    for id in ids {
        let from: Option<Option<i64>> = c
            .query_row(
                "SELECT category_id FROM banks WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        match from {
            None => return Ok(Err(CategoryError::BankNotFound)),
            Some(f) => {
                if !sources.contains(&f) {
                    sources.push(f);
                }
            }
        }
    }
    // Plan every group before writing: the groups the banks leave keep their order without them
    let mut leaving: Vec<(Option<i64>, Vec<i64>)> = Vec::new();
    for from in sources.into_iter().filter(|f| *f != category) {
        let rest: Vec<i64> = group_ids(c, from)?
            .into_iter()
            .filter(|b| !moved.contains(b))
            .collect();
        leaving.push((from, rest));
    }
    let mut target: Vec<i64> = group_ids(c, category)?
        .into_iter()
        .filter(|b| !moved.contains(b))
        .collect();
    target.extend_from_slice(ids);
    place_banks(c, category, &target)?;
    for (from, rest) in leaving {
        place_banks(c, from, &rest)?;
    }
    Ok(Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &str) -> Option<String> {
        clean_category_name(s).ok()
    }

    #[test]
    fn names_are_cleaned_and_checked() {
        assert_eq!(name("  Science  "), Some("Science".into()));
        assert_eq!(name("Everyday \t\n  life"), Some("Everyday life".into()));
        assert_eq!(name("日本語 の 分類"), Some("日本語 の 分類".into()));
        assert_eq!(name(""), None);
        assert_eq!(name("   \t "), None);
        assert_eq!(name(&"a".repeat(60)), Some("a".repeat(60)));
        assert_eq!(name(&"a".repeat(61)), None);
        // the length counts characters, not bytes
        assert_eq!(name(&"é".repeat(60)).map(|s| s.chars().count()), Some(60));
        assert_eq!(name("bell\u{7}"), None);
        assert_eq!(name("zero\u{200B}width"), None);
        assert_eq!(name("rtl\u{202E}"), None);
        assert_eq!(name("bom\u{FEFF}"), None);
        assert_eq!(name("nul\u{0}"), None);
    }

    #[test]
    fn same_set_wants_every_id_once() {
        assert!(same_set(&[3, 1, 2], &[1, 2, 3]));
        assert!(same_set(&[], &[]));
        assert!(!same_set(&[1, 1, 2], &[1, 2, 3]));
        assert!(!same_set(&[1, 2], &[1, 2, 3]));
        assert!(!same_set(&[1, 2, 4], &[1, 2, 3]));
        assert!(!same_set(&[1, 2, 3, 3], &[1, 2, 3]));
    }
}
