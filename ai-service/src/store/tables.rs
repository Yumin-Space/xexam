//! The `settings`, `secrets` and `limits` rows.
use crate::limits::{Limits, DEFS};
use crate::secret::{Sealed, SecretName};
use crate::settings::{Format, GatewayAuth, KeyRevs, Provider, Settings};
use rusqlite::{params, Connection, OptionalExtension};

fn bad_value(col: usize) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(col, "enum".into(), rusqlite::types::Type::Text)
}

/// Reads the settings row.
pub fn load_settings(c: &Connection) -> rusqlite::Result<Settings> {
    c.query_row(
        "SELECT provider, base_url, format, gateway_auth, model, manual_model, usage_option, max_tokens, enabled,
                verified_at, verified_fp, retention_days, updated_at, updated_by
         FROM settings WHERE id = 1",
        [],
        |r| {
            let provider: String = r.get(0)?;
            let format: String = r.get(2)?;
            let auth: String = r.get(3)?;
            Ok(Settings {
                provider: Provider::parse(&provider).ok_or_else(|| bad_value(0))?,
                base_url: r.get(1)?,
                format: Format::parse(&format).ok_or_else(|| bad_value(2))?,
                gateway_auth: GatewayAuth::parse(&auth).ok_or_else(|| bad_value(3))?,
                model: r.get(4)?,
                manual_model: r.get::<_, i64>(5)? != 0,
                usage_option: r.get::<_, i64>(6)? != 0,
                max_tokens: r.get(7)?,
                enabled: r.get::<_, i64>(8)? != 0,
                verified_at: r.get(9)?,
                verified_fp: r.get(10)?,
                retention_days: r.get(11)?,
                updated_at: r.get(12)?,
                updated_by: r.get(13)?,
            })
        },
    )
}

/// Writes the whole settings row.
pub fn save_settings(c: &Connection, s: &Settings) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE settings SET provider = ?1, base_url = ?2, format = ?3, gateway_auth = ?4, model = ?5,
                manual_model = ?6, usage_option = ?7, max_tokens = ?8, enabled = ?9, verified_at = ?10,
                verified_fp = ?11, retention_days = ?12, updated_at = ?13, updated_by = ?14
         WHERE id = 1",
        params![
            s.provider.as_str(),
            s.base_url,
            s.format.as_str(),
            s.gateway_auth.as_str(),
            s.model,
            i64::from(s.manual_model),
            i64::from(s.usage_option),
            s.max_tokens,
            i64::from(s.enabled),
            s.verified_at,
            s.verified_fp,
            s.retention_days,
            s.updated_at,
            s.updated_by,
        ],
    )?;
    Ok(())
}

/// One saved key (sealed). No `Debug`.
pub struct SecretRow {
    pub name: SecretName,
    pub key_id: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub hint: String,
    pub rev: i64,
}

pub fn load_secrets(c: &Connection) -> rusqlite::Result<Vec<SecretRow>> {
    let mut st =
        c.prepare("SELECT name, key_id, nonce, ciphertext, hint, rev FROM secrets ORDER BY name")?;
    let rows = st.query_map([], |r| {
        let name: String = r.get(0)?;
        Ok(SecretRow {
            name: SecretName::from_db(&name).ok_or_else(|| bad_value(0))?,
            key_id: r.get(1)?,
            nonce: r.get(2)?,
            ciphertext: r.get(3)?,
            hint: r.get(4)?,
            rev: r.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn key_revs(rows: &[SecretRow]) -> KeyRevs {
    let mut k = KeyRevs::default();
    for r in rows {
        match r.name {
            SecretName::ApiKey => k.api = Some(r.rev),
            SecretName::GatewayKey => k.gateway = Some(r.rev),
        }
    }
    k
}

/// Writes (replaces) a sealed key with a new revision; returns the revision.
pub fn put_secret(
    c: &Connection,
    name: SecretName,
    key_id: &str,
    sealed: &Sealed,
    hint: &str,
    now: i64,
) -> rusqlite::Result<i64> {
    // One counter for both slots, so a revision is never reused (a removed and re-entered key is a new revision)
    let last: i64 = c
        .query_row("SELECT value FROM meta WHERE key = 'secret_rev'", [], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let rev = last + 1;
    c.execute(
        "INSERT INTO meta (key, value) VALUES ('secret_rev', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [rev.to_string()],
    )?;
    c.execute(
        "INSERT INTO secrets (name, key_id, nonce, ciphertext, hint, rev, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(name) DO UPDATE SET key_id = excluded.key_id, nonce = excluded.nonce,
             ciphertext = excluded.ciphertext, hint = excluded.hint, rev = excluded.rev,
             updated_at = excluded.updated_at",
        params![
            name.db(),
            key_id,
            sealed.nonce.as_slice(),
            sealed.ciphertext,
            hint,
            rev,
            now
        ],
    )?;
    Ok(rev)
}

pub fn delete_secret(c: &Connection, name: SecretName) -> rusqlite::Result<bool> {
    Ok(c.execute("DELETE FROM secrets WHERE name = ?1", [name.db()])? > 0)
}

pub fn load_limits(c: &Connection) -> rusqlite::Result<Limits> {
    let cols: Vec<&str> = DEFS.iter().map(|d| d.column).collect();
    let sql = format!("SELECT {} FROM limits WHERE id = 1", cols.join(", "));
    c.query_row(&sql, [], |r| {
        Ok(Limits {
            per_user_running: r.get(0)?,
            global_running: r.get(1)?,
            chats_per_user: r.get(2)?,
            messages_per_chat: r.get(3)?,
            memories_per_user: r.get(4)?,
            memory_chars: r.get(5)?,
            message_chars: r.get(6)?,
        })
    })
}

pub fn save_limits(c: &Connection, l: &Limits, by: &str, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE limits SET per_user_running = ?1, global_running = ?2, chats_per_user = ?3,
                messages_per_chat = ?4, memories_per_user = ?5, memory_chars = ?6, message_chars = ?7,
                updated_at = ?8, updated_by = ?9
         WHERE id = 1",
        params![
            l.per_user_running,
            l.global_running,
            l.chats_per_user,
            l.messages_per_chat,
            l.memories_per_user,
            l.memory_chars,
            l.message_chars,
            now,
            by
        ],
    )?;
    Ok(())
}
