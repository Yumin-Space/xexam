//! The `geo_settings` row (schema 8): the settings of the external IP-location lookup, with the sealed key. A row exists
//! only after a passing "test and save".
use crate::geo::mapping::Mapping;
use crate::geo::template::ProviderCfg;
use rusqlite::{params, Connection, OptionalExtension};

/// The preset ids the table accepts (display only: the other columns are what is used).
pub const PRESETS: [&str; 6] = ["ipinfo", "ipapi", "ipapico", "ipwhois", "ipsb", "custom"];

/// A key sealed with the AI master key under `secret::GEO_KEY_AAD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedKey {
    pub key_id: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    /// `…` + the last 4 characters of a key of at least 12, else ""
    pub hint: String,
}

/// The saved settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeoRow {
    pub preset: String,
    pub cfg: ProviderCfg,
    pub key: Option<SealedKey>,
    pub tested_at: i64,
    pub updated_at: i64,
    pub updated_by: Option<i64>,
}

/// Reads the settings, or `None` when none are saved.
pub fn load(c: &Connection) -> rusqlite::Result<Option<GeoRow>> {
    c.query_row(
        "SELECT preset, url_template, key_header_name, key_header_value, map_country, map_country_code, map_region,
                map_city, ok_path, ok_value, error_path, message_path, key_id, key_nonce, key_ciphertext, key_hint,
                tested_at, updated_at, updated_by
         FROM geo_settings WHERE id = 1",
        [],
        |r| {
            let key_id: Option<String> = r.get(12)?;
            let nonce: Option<Vec<u8>> = r.get(13)?;
            let ct: Option<Vec<u8>> = r.get(14)?;
            let key = match (key_id, nonce, ct) {
                (Some(key_id), Some(nonce), Some(ciphertext)) => Some(SealedKey {
                    key_id,
                    nonce,
                    ciphertext,
                    hint: r.get(15)?,
                }),
                _ => None,
            };
            Ok(GeoRow {
                preset: r.get(0)?,
                cfg: ProviderCfg {
                    url_template: r.get(1)?,
                    key_header_name: r.get(2)?,
                    key_header_value: r.get(3)?,
                    mapping: Mapping {
                        country: r.get(4)?,
                        country_code: r.get(5)?,
                        region: r.get(6)?,
                        city: r.get(7)?,
                        ok_path: r.get(8)?,
                        ok_value: r.get(9)?,
                        error_path: r.get(10)?,
                        message_path: r.get(11)?,
                    },
                },
                key,
                tested_at: r.get(16)?,
                updated_at: r.get(17)?,
                updated_by: r.get(18)?,
            })
        },
    )
    .optional()
}

/// Writes the whole row (replacing the saved one).
pub fn save(c: &Connection, row: &GeoRow) -> rusqlite::Result<()> {
    let m = &row.cfg.mapping;
    let (key_id, nonce, ct, hint) = match &row.key {
        Some(k) => (
            Some(k.key_id.as_str()),
            Some(k.nonce.as_slice()),
            Some(k.ciphertext.as_slice()),
            k.hint.as_str(),
        ),
        None => (None, None, None, ""),
    };
    c.execute(
        "INSERT OR REPLACE INTO geo_settings
           (id, preset, url_template, key_header_name, key_header_value, map_country, map_country_code, map_region,
            map_city, ok_path, ok_value, error_path, message_path, key_id, key_nonce, key_ciphertext, key_hint,
            tested_at, updated_at, updated_by)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            row.preset,
            row.cfg.url_template,
            row.cfg.key_header_name,
            row.cfg.key_header_value,
            m.country,
            m.country_code,
            m.region,
            m.city,
            m.ok_path,
            m.ok_value,
            m.error_path,
            m.message_path,
            key_id,
            nonce,
            ct,
            hint,
            row.tested_at,
            row.updated_at,
            row.updated_by,
        ],
    )?;
    Ok(())
}

/// Deletes the saved settings and the key with them.
pub fn delete(c: &Connection) -> rusqlite::Result<()> {
    c.execute("DELETE FROM geo_settings", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::JournalMode;
    use crate::store::Store;

    fn row(key: Option<SealedKey>) -> GeoRow {
        GeoRow {
            preset: "custom".into(),
            cfg: ProviderCfg {
                url_template: "https://provider.example/{ip}/json?token={key}".into(),
                key_header_name: "X-Key".into(),
                key_header_value: "{key}".into(),
                mapping: Mapping {
                    country: "c".into(),
                    country_code: "cc".into(),
                    region: "r".into(),
                    city: "ci".into(),
                    ok_path: "status".into(),
                    ok_value: "success".into(),
                    error_path: "error".into(),
                    message_path: "message".into(),
                },
            },
            key,
            tested_at: 1_763_200_000,
            updated_at: 1_763_200_001,
            updated_by: Some(7),
        }
    }

    #[test]
    fn save_load_replace_and_delete() {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::open(dir.path(), JournalMode::Delete).expect("store");
        assert_eq!(store.read(load).expect("load"), None);
        let sealed = SealedKey {
            key_id: "0123456789abcdef".into(),
            nonce: vec![1; 24],
            ciphertext: vec![2, 3, 4],
            hint: "…cdef".into(),
        };
        let a = row(Some(sealed));
        store.write(|tx| save(tx, &a)).expect("save");
        assert_eq!(store.read(load).expect("load"), Some(a.clone()));
        // a second save replaces the row (still one row), here without a key
        let mut b = row(None);
        b.preset = "ipinfo".into();
        store.write(|tx| save(tx, &b)).expect("save");
        assert_eq!(store.read(load).expect("load"), Some(b));
        let n: i64 = store
            .read(|c| c.query_row("SELECT COUNT(*) FROM geo_settings", [], |r| r.get(0)))
            .expect("count");
        assert_eq!(n, 1);
        store.write(|tx| delete(tx)).expect("delete");
        assert_eq!(store.read(load).expect("load"), None);
    }

    #[test]
    fn the_table_refuses_an_unknown_preset() {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::open(dir.path(), JournalMode::Delete).expect("store");
        let mut bad = row(None);
        bad.preset = "nope".into();
        assert!(store.write(|tx| save(tx, &bad)).is_err());
        for p in PRESETS {
            let mut ok = row(None);
            ok.preset = p.into();
            store.write(|tx| save(tx, &ok)).expect(p);
        }
    }
}
