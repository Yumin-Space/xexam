//! The `settings` and `limits` rows, and the [`SecretRow`] type of `connection_secrets` (read by `store::conn`). The old
//! single-connection items of `settings` and the `secrets` table were dropped by AI schema 10.
use crate::limits::{Limits, DEFS};
use crate::secret::SecretName;
use crate::settings::Settings;
use rusqlite::Connection;

/// Reads the settings row.
pub fn load_settings(c: &Connection) -> rusqlite::Result<Settings> {
    c.query_row(
        "SELECT enabled, retention_days, updated_at, updated_by FROM settings WHERE id = 1",
        [],
        |r| {
            Ok(Settings {
                enabled: r.get::<_, i64>(0)? != 0,
                retention_days: r.get(1)?,
                updated_at: r.get(2)?,
                updated_by: r.get(3)?,
            })
        },
    )
}

/// Switches AI on or off (does not touch `updated_at` / `updated_by`); `true` when the stored value changed.
pub fn set_enabled(c: &Connection, on: bool) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "UPDATE settings SET enabled = ?1 WHERE id = 1 AND enabled <> ?1",
        [i64::from(on)],
    )? > 0)
}

/// Sets the retention period with its author and time; `true` when the stored value changed.
pub fn set_retention(c: &Connection, days: i64, by: &str, now: i64) -> rusqlite::Result<bool> {
    Ok(c.execute(
        "UPDATE settings SET retention_days = ?1, updated_at = ?2, updated_by = ?3
         WHERE id = 1 AND retention_days <> ?1",
        rusqlite::params![days, now, by],
    )? > 0)
}

/// One saved key (sealed). No `Debug`.
pub struct SecretRow {
    pub name: SecretName,
    pub key_id: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub hint: String,
    pub rev: i64,
    /// 1 = sealed with the AAD of the old single connection (`...-v1:<name>`), 2 = with the connection id
    pub aad_ver: i64,
}

/// Reads the limits row by column name (never by position): every field of [`DEFS`] comes from its own column.
pub fn load_limits(c: &Connection) -> rusqlite::Result<Limits> {
    let cols: Vec<&str> = DEFS.iter().map(|d| d.column).collect();
    let sql = format!("SELECT {} FROM limits WHERE id = 1", cols.join(", "));
    c.query_row(&sql, [], |r| {
        let mut l = Limits::default();
        for d in &DEFS {
            l.set(d.key, r.get(d.column)?);
        }
        Ok(l)
    })
}

/// Writes the limits row: one `column = ?n` per entry of [`DEFS`], bound in the same order.
pub fn save_limits(c: &Connection, l: &Limits, by: &str, now: i64) -> rusqlite::Result<()> {
    let sets: Vec<String> = DEFS
        .iter()
        .enumerate()
        .map(|(i, d)| format!("{} = ?{}", d.column, i + 1))
        .collect();
    let n = DEFS.len();
    let sql = format!(
        "UPDATE limits SET {}, updated_at = ?{}, updated_by = ?{} WHERE id = 1",
        sets.join(", "),
        n + 1,
        n + 2
    );
    let mut values: Vec<rusqlite::types::Value> = DEFS
        .iter()
        .map(|d| rusqlite::types::Value::Integer(l.get(d.key)))
        .collect();
    values.push(rusqlite::types::Value::Integer(now));
    values.push(rusqlite::types::Value::Text(by.to_string()));
    c.execute(&sql, rusqlite::params_from_iter(values))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::JournalMode;
    use crate::store::Store;

    #[test]
    fn limits_round_trip_keeps_every_field_in_its_own_field() {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::open(dir.path(), JournalMode::Delete).expect("store");
        // ten distinct values inside every range; a positional mix-up changes at least one field
        let mut l = Limits::default();
        let values = [2, 15, 150, 330, 44, 222, 7, 33, 444, 12_345];
        for (d, v) in DEFS.iter().zip(values) {
            assert!((d.min..=d.max).contains(&v), "{}", d.key);
            l.set(d.key, v);
        }
        store
            .write(|tx| save_limits(tx, &l, "root", 1_700_000_000))
            .expect("save");
        let back = store.read(load_limits).expect("load");
        assert_eq!(back, l);
        for (d, v) in DEFS.iter().zip(values) {
            assert_eq!(back.get(d.key), v, "{}", d.key);
        }
        let (at, by): (i64, String) = store
            .read(|c| {
                c.query_row(
                    "SELECT updated_at, updated_by FROM limits WHERE id = 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            })
            .expect("row");
        assert_eq!((at, by.as_str()), (1_700_000_000, "root"));
    }
}
