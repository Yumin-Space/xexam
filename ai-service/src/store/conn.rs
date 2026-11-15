//! Connections, their sealed keys, the model pool and the six model slots (schema 7). Every function takes the
//! connection or transaction of the caller; the admin routes run the writes inside `Store::write`.
//!
//! Keys are only ever stored sealed (`connection_secrets`); nothing here sees a plain key. Slot names are the lower-case
//! strings of `route::Slot::as_str` (`default`, `chat`, `study`, `summary`, `generate`, `import`).
use crate::effort::Effort;
use crate::secret::{Sealed, SecretName};
use crate::store::tables::SecretRow;
use rusqlite::{params, Connection, OptionalExtension};

/// The model slots, in the order `slots` returns them.
pub const SLOT_NAMES: [&str; 6] = ["default", "chat", "study", "summary", "generate", "import"];

/// One connection as listed (never a key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnRow {
    pub id: i64,
    /// +1 on every save; the optimistic lock and the cache key
    pub rev: i64,
    pub name: String,
    /// 'openai' | 'anthropic' | 'gateway'
    pub provider: String,
    pub base_url: String,
    /// 'openai' | 'anthropic'
    pub format: String,
    /// 'bearer' | 'x-api-key' | 'cf-aig-authorization'
    pub gateway_auth: String,
    /// 'ok' | 'broken'
    pub status: String,
    pub error_code: Option<String>,
    pub lists_models: bool,
    pub tested_at: Option<i64>,
    pub sort: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub updated_by: String,
}

/// A connection to create (status `ok`, rev 1, sorted last).
#[derive(Debug, Clone)]
pub struct NewConn<'a> {
    pub name: &'a str,
    pub provider: &'a str,
    pub base_url: &'a str,
    pub format: &'a str,
    pub gateway_auth: &'a str,
    pub lists_models: bool,
    pub by: &'a str,
    pub now: i64,
}

/// The fields a save of a connection writes.
#[derive(Debug, Clone)]
pub struct ConnChange<'a> {
    pub name: &'a str,
    pub provider: &'a str,
    pub base_url: &'a str,
    pub format: &'a str,
    pub gateway_auth: &'a str,
    pub lists_models: bool,
    pub by: &'a str,
    pub now: i64,
}

/// One model of the pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolRow {
    pub id: i64,
    pub connection_id: i64,
    pub model: String,
    pub manual: bool,
    /// 'ok' | 'untested' | 'failed'
    pub status: String,
    pub error_code: Option<String>,
    pub usage_option: bool,
    /// 4096 | 8192
    pub max_tokens: i64,
    pub context: Option<i64>,
    pub latency_ms: Option<i64>,
    pub tested_at: Option<i64>,
}

/// A pool entry to insert, or to update when `(connection_id, model)` exists.
#[derive(Debug, Clone)]
pub struct NewPool<'a> {
    pub connection_id: i64,
    pub model: &'a str,
    pub manual: bool,
    pub status: &'a str,
    pub error_code: Option<&'a str>,
    pub usage_option: bool,
    pub max_tokens: i64,
    pub context: Option<i64>,
    pub latency_ms: Option<i64>,
    pub tested_at: Option<i64>,
}

/// A sealed key to store for a connection.
pub struct NewConnSecret<'a> {
    pub name: SecretName,
    pub key_id: &'a str,
    pub sealed: &'a Sealed,
    pub hint: &'a str,
    /// 1 = AAD without the connection id (migrated rows), 2 = with it (every new seal)
    pub aad_ver: i64,
    pub now: i64,
}

const CONN_COLS: &str =
    "id, rev, name, provider, base_url, format, gateway_auth, status, error_code, lists_models,
     tested_at, sort, created_at, updated_at, updated_by";

fn conn_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<ConnRow> {
    Ok(ConnRow {
        id: r.get(0)?,
        rev: r.get(1)?,
        name: r.get(2)?,
        provider: r.get(3)?,
        base_url: r.get(4)?,
        format: r.get(5)?,
        gateway_auth: r.get(6)?,
        status: r.get(7)?,
        error_code: r.get(8)?,
        lists_models: r.get::<_, i64>(9)? != 0,
        tested_at: r.get(10)?,
        sort: r.get(11)?,
        created_at: r.get(12)?,
        updated_at: r.get(13)?,
        updated_by: r.get(14)?,
    })
}

/// Every connection, in display order (`sort`, then id).
pub fn connections(c: &Connection) -> rusqlite::Result<Vec<ConnRow>> {
    let mut st = c.prepare(&format!(
        "SELECT {CONN_COLS} FROM connections ORDER BY sort, id"
    ))?;
    let rows = st.query_map([], conn_from)?;
    rows.collect()
}

pub fn connection(c: &Connection, id: i64) -> rusqlite::Result<Option<ConnRow>> {
    c.query_row(
        &format!("SELECT {CONN_COLS} FROM connections WHERE id = ?1"),
        [id],
        conn_from,
    )
    .optional()
}

/// `true` when another connection (not `except`) already has this name (case-insensitive, like the unique index).
pub fn name_taken(c: &Connection, name: &str, except: Option<i64>) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT EXISTS(SELECT 1 FROM connections WHERE name = ?1 COLLATE NOCASE AND id <> COALESCE(?2, -1))",
        params![name, except],
        |r| r.get(0),
    )
}

/// Creates a connection (status `ok`, rev 1, last in the order); returns its id. A duplicate name fails with a
/// constraint error (check [`name_taken`] first for a friendly answer).
pub fn insert_connection(c: &Connection, n: &NewConn<'_>) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO connections (name, provider, base_url, format, gateway_auth, lists_models, sort, created_at,
                                  updated_at, updated_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, (SELECT COALESCE(MAX(sort), -1) + 1 FROM connections), ?7, ?7, ?8)",
        params![
            n.name,
            n.provider,
            n.base_url,
            n.format,
            n.gateway_auth,
            i64::from(n.lists_models),
            n.now,
            n.by
        ],
    )?;
    Ok(c.last_insert_rowid())
}

/// Saves a connection when its revision is still `rev`; the revision goes up by one. `false` = the revision did not
/// match (another administrator saved first: `ai.conflict`).
pub fn update_connection(
    c: &Connection,
    id: i64,
    rev: i64,
    n: &ConnChange<'_>,
) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "UPDATE connections SET name = ?3, provider = ?4, base_url = ?5, format = ?6, gateway_auth = ?7,
                lists_models = ?8, rev = rev + 1, updated_at = ?9, updated_by = ?10
         WHERE id = ?1 AND rev = ?2",
        params![
            id,
            rev,
            n.name,
            n.provider,
            n.base_url,
            n.format,
            n.gateway_auth,
            i64::from(n.lists_models),
            n.now,
            n.by
        ],
    )? > 0)
}

/// Deletes a connection with its keys and pool models (cascade); the slots that pointed at one of its models become
/// empty and every chat pick of the connection is cleared (the chat is back on its default). `usage` / `gen_calls` rows
/// keep the id (no foreign key).
pub fn delete_connection(c: &Connection, id: i64) -> rusqlite::Result<()> {
    c.execute("DELETE FROM connections WHERE id = ?1", [id])?;
    c.execute(
        "UPDATE chats SET pick_conn = NULL, pick_model = NULL WHERE pick_conn = ?1",
        [id],
    )?;
    Ok(())
}

/// Sets the display order: `ids[i]` gets `sort = i`. Ids that are not connections are ignored; connections not listed
/// keep their relative order after the listed ones.
pub fn order_connections(c: &Connection, ids: &[i64]) -> rusqlite::Result<()> {
    let n = i64::try_from(ids.len()).unwrap_or(i64::MAX);
    for (i, id) in ids.iter().enumerate() {
        c.execute(
            "UPDATE connections SET sort = ?2 WHERE id = ?1",
            params![id, i64::try_from(i).unwrap_or(i64::MAX)],
        )?;
    }
    let mut st = c.prepare("SELECT id FROM connections ORDER BY sort, id")?;
    let rest: Vec<i64> = st
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|id| !ids.contains(id))
        .collect();
    for (i, id) in rest.iter().enumerate() {
        c.execute(
            "UPDATE connections SET sort = ?2 WHERE id = ?1",
            params![id, n + i64::try_from(i).unwrap_or(0)],
        )?;
    }
    Ok(())
}

/// Records the outcome of a use or test: `status` 'ok' / 'broken', the failing code while broken. `tested_at` is
/// kept when None.
pub fn set_connection_status(
    c: &Connection,
    id: i64,
    status: &str,
    code: Option<&str>,
    tested_at: Option<i64>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE connections SET status = ?2, error_code = ?3, tested_at = COALESCE(?4, tested_at) WHERE id = ?1",
        params![id, status, code, tested_at],
    )?;
    Ok(())
}

/// The sealed keys of a connection (`api_key`, `gateway_key`), by name.
pub fn conn_secrets(c: &Connection, id: i64) -> rusqlite::Result<Vec<SecretRow>> {
    let mut st = c.prepare(
        "SELECT name, key_id, nonce, ciphertext, hint, rev, aad_ver FROM connection_secrets
         WHERE connection_id = ?1 ORDER BY name",
    )?;
    let rows = st.query_map([id], |r| {
        let name: String = r.get(0)?;
        Ok(SecretRow {
            name: SecretName::from_db(&name).ok_or_else(|| {
                rusqlite::Error::InvalidColumnType(0, "enum".into(), rusqlite::types::Type::Text)
            })?,
            key_id: r.get(1)?,
            nonce: r.get(2)?,
            ciphertext: r.get(3)?,
            hint: r.get(4)?,
            rev: r.get(5)?,
            aad_ver: r.get(6)?,
        })
    })?;
    rows.collect()
}

/// Writes (replaces) a sealed key of a connection with a new revision from the one counter that the old `secrets`
/// table used too (a revision is never reused); returns the revision.
pub fn put_conn_secret(c: &Connection, id: i64, s: &NewConnSecret<'_>) -> rusqlite::Result<i64> {
    let last: i64 = c
        .query_row("SELECT value FROM meta WHERE key = 'secret_rev'", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // the migration copies old revisions as they are, so the counter is never below the highest stored one
    let top: i64 = c.query_row(
        "SELECT COALESCE(MAX(rev), 0) FROM connection_secrets",
        [],
        |r| r.get(0),
    )?;
    let rev = last.max(top) + 1;
    c.execute(
        "INSERT INTO meta (key, value) VALUES ('secret_rev', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [rev.to_string()],
    )?;
    c.execute(
        "INSERT INTO connection_secrets (connection_id, name, key_id, nonce, ciphertext, hint, rev, aad_ver, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(connection_id, name) DO UPDATE SET key_id = excluded.key_id, nonce = excluded.nonce,
             ciphertext = excluded.ciphertext, hint = excluded.hint, rev = excluded.rev,
             aad_ver = excluded.aad_ver, updated_at = excluded.updated_at",
        params![
            id,
            s.name.db(),
            s.key_id,
            s.sealed.nonce.as_slice(),
            s.sealed.ciphertext,
            s.hint,
            rev,
            s.aad_ver,
            s.now
        ],
    )?;
    Ok(rev)
}

pub fn delete_conn_secret(c: &Connection, id: i64, name: SecretName) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "DELETE FROM connection_secrets WHERE connection_id = ?1 AND name = ?2",
        params![id, name.db()],
    )? > 0)
}

const POOL_COLS: &str =
    "id, connection_id, model, manual, status, error_code, usage_option, max_tokens, context, latency_ms, tested_at";

fn pool_from(r: &rusqlite::Row<'_>) -> rusqlite::Result<PoolRow> {
    Ok(PoolRow {
        id: r.get(0)?,
        connection_id: r.get(1)?,
        model: r.get(2)?,
        manual: r.get::<_, i64>(3)? != 0,
        status: r.get(4)?,
        error_code: r.get(5)?,
        usage_option: r.get::<_, i64>(6)? != 0,
        max_tokens: r.get(7)?,
        context: r.get(8)?,
        latency_ms: r.get(9)?,
        tested_at: r.get(10)?,
    })
}

/// The whole pool, by connection order then model id.
pub fn pool(c: &Connection) -> rusqlite::Result<Vec<PoolRow>> {
    let mut st = c.prepare(&format!(
        "SELECT {} FROM models m JOIN connections k ON k.id = m.connection_id ORDER BY k.sort, k.id, m.id",
        POOL_COLS
            .split(", ")
            .map(|c| format!("m.{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))?;
    let rows = st.query_map([], pool_from)?;
    rows.collect()
}

pub fn pool_row(c: &Connection, id: i64) -> rusqlite::Result<Option<PoolRow>> {
    c.query_row(
        &format!("SELECT {POOL_COLS} FROM models WHERE id = ?1"),
        [id],
        pool_from,
    )
    .optional()
}

/// Adds a model to the pool, or refreshes the entry of the same `(connection, model)`; returns the pool id.
pub fn upsert_pool(c: &Connection, p: &NewPool<'_>) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO models (connection_id, model, manual, status, error_code, usage_option, max_tokens, context,
                             latency_ms, tested_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(connection_id, model) DO UPDATE SET manual = excluded.manual, status = excluded.status,
             error_code = excluded.error_code, usage_option = excluded.usage_option,
             max_tokens = excluded.max_tokens, context = excluded.context, latency_ms = excluded.latency_ms,
             tested_at = excluded.tested_at",
        params![
            p.connection_id,
            p.model,
            i64::from(p.manual),
            p.status,
            p.error_code,
            i64::from(p.usage_option),
            p.max_tokens,
            p.context,
            p.latency_ms,
            p.tested_at
        ],
    )?;
    c.query_row(
        "SELECT id FROM models WHERE connection_id = ?1 AND model = ?2",
        params![p.connection_id, p.model],
        |r| r.get(0),
    )
}

pub fn set_pool_status(
    c: &Connection,
    id: i64,
    status: &str,
    code: Option<&str>,
    latency_ms: Option<i64>,
    tested_at: Option<i64>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE models SET status = ?2, error_code = ?3, latency_ms = ?4, tested_at = ?5 WHERE id = ?1",
        params![id, status, code, latency_ms, tested_at],
    )?;
    Ok(())
}

/// Removes a model from the pool; a slot that pointed at it becomes empty.
pub fn remove_pool(c: &Connection, id: i64) -> rusqlite::Result<()> {
    c.execute("DELETE FROM models WHERE id = ?1", [id])?;
    Ok(())
}

/// The pool id of every slot, in [`SLOT_NAMES`] order; None = empty.
pub fn slots(c: &Connection) -> rusqlite::Result<[Option<i64>; 6]> {
    let mut out = [None; 6];
    let mut st = c.prepare("SELECT slot, model_id FROM model_slots")?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
    })?;
    for row in rows {
        let (name, id) = row?;
        if let Some(i) = SLOT_NAMES.iter().position(|s| *s == name) {
            out[i] = id;
        }
    }
    Ok(out)
}

/// Points a slot at a pool model (None = empty). An unknown slot name or a pool id that is not in the pool is refused
/// (`QueryReturnedNoRows` / the foreign key error).
pub fn set_slot(
    c: &Connection,
    slot: &str,
    pool_id: Option<i64>,
    by: &str,
) -> rusqlite::Result<()> {
    let now = crate::util::unix_now();
    if !SLOT_NAMES.contains(&slot) {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    c.execute(
        "INSERT INTO model_slots (slot, model_id, updated_at, updated_by) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(slot) DO UPDATE SET model_id = excluded.model_id, updated_at = excluded.updated_at,
             updated_by = excluded.updated_by",
        params![slot, pool_id, now, by],
    )?;
    Ok(())
}

/// The global feature switches (`settings.features_off`, bits of `features.rs`).
pub fn global_features_off(c: &Connection) -> rusqlite::Result<i64> {
    c.query_row("SELECT features_off FROM settings WHERE id = 1", [], |r| {
        r.get(0)
    })
}

/// Stores the global feature switches (0 ..= 63).
pub fn set_features_off(c: &Connection, bits: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE settings SET features_off = ?1 WHERE id = 1",
        [bits.clamp(0, 63)],
    )?;
    Ok(())
}

/// The thinking strength of every slot, in [`SLOT_NAMES`] order; `None` = send nothing (also for an unreadable value).
pub fn efforts(c: &Connection) -> rusqlite::Result<[Option<Effort>; 6]> {
    let mut out = [None; 6];
    let mut st = c.prepare("SELECT slot, effort FROM model_slots")?;
    let rows = st.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (name, tag) = row?;
        if let Some(i) = SLOT_NAMES.iter().position(|s| *s == name) {
            out[i] = tag.as_deref().and_then(Effort::parse_tag);
        }
    }
    Ok(out)
}

/// Sets a slot's thinking strength (`None` = default). An unknown slot name is refused (`QueryReturnedNoRows`, like
/// [`set_slot`]); the slot's model is left alone.
pub fn set_effort(
    c: &Connection,
    slot: &str,
    effort: Option<Effort>,
    by: &str,
) -> rusqlite::Result<()> {
    let now = crate::util::unix_now();
    if !SLOT_NAMES.contains(&slot) {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    c.execute(
        "INSERT INTO model_slots (slot, model_id, effort, updated_at, updated_by) VALUES (?1, NULL, ?2, ?3, ?4)
         ON CONFLICT(slot) DO UPDATE SET effort = excluded.effort, updated_at = excluded.updated_at,
             updated_by = excluded.updated_by",
        params![slot, effort.map(Effort::tag), now, by],
    )?;
    Ok(())
}
