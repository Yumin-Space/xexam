//! Settings (v0.5): one row per changed key, `value` = the JSON text of the value (`"dark"`, `true`). Since v0.6 they are per
//! user, in `user_settings` (the v0.5 `settings` table was copied to the first administrator by the schema-v3 migration).
//! Reading merges the stored values over the defaults; stored values that are not valid (e.g. written by a newer version) are ignored.
use super::super::pyjson;
use super::{invalid_input, run_tx, Store};
use crate::error::DbError;
use crate::models::{setting_default, setting_valid, RESETTABLE_SETTINGS, SETTING_KEYS};
use rusqlite::{params, Connection};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// `{"theme", "uiLang", "autoAdvance", "questionLang"}` in that order.
fn settings_json(c: &Connection, user: i64) -> Result<Value, DbError> {
    let stored: HashMap<String, String> = {
        let mut st = c.prepare("SELECT key, value FROM user_settings WHERE user_id=?1")?;
        let rows = st.query_map(params![user], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut out = Map::new();
    for key in SETTING_KEYS {
        let value = stored
            .get(key)
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .filter(|v| setting_valid(key, v))
            .or_else(|| setting_default(key))
            .unwrap_or(Value::Null);
        out.insert(key.to_string(), value);
    }
    Ok(Value::Object(out))
}

impl Store {
    /// `GET /api/settings` of `user`
    pub fn settings(&self, user: i64) -> Result<Value, DbError> {
        let g = self.lock();
        settings_json(&g, user)
    }

    /// `POST /api/settings`: stores every `(key, value)` in one transaction and returns the full settings. Every pair must be a
    /// known key with a valid value (the API validates first); otherwise nothing is written and an error is returned.
    pub fn update_settings(
        &self,
        user: i64,
        changes: &[(String, Value)],
    ) -> Result<Value, DbError> {
        if let Some((key, _)) = changes.iter().find(|(k, v)| !setting_valid(k, v)) {
            return Err(invalid_input(&format!("invalid value for setting {key:?}")));
        }
        let texts = changes
            .iter()
            .map(|(k, v)| Ok((k.as_str(), pyjson::to_string(v)?)))
            .collect::<Result<Vec<_>, DbError>>()?;
        let g = self.lock();
        if !texts.is_empty() {
            run_tx(&g, |c| {
                super::users::ensure_active(c, user)?;
                let mut up = c.prepare(
                    "INSERT INTO user_settings(user_id, key, value) VALUES(?1, ?2, ?3) \
                     ON CONFLICT(user_id, key) DO UPDATE SET value=excluded.value",
                )?;
                for (key, text) in &texts {
                    up.execute(params![user, key, text])?;
                }
                Ok(())
            })?;
        }
        settings_json(&g, user)
    }

    /// `POST /api/settings/reset`: back to the defaults for theme, UI language and auto-advance (`questionLang` is kept).
    pub fn reset_settings(&self, user: i64) -> Result<Value, DbError> {
        let g = self.lock();
        run_tx(&g, |c| {
            super::users::ensure_active(c, user)?;
            let mut del = c.prepare("DELETE FROM user_settings WHERE user_id=?1 AND key=?2")?;
            for key in RESETTABLE_SETTINGS {
                del.execute(params![user, key])?;
            }
            Ok(())
        })?;
        settings_json(&g, user)
    }
}
