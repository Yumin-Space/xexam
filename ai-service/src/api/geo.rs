//! The administrators' routes of the external IP-location lookup (spec 3.3): `GET /api/ai/admin/geo` and the POSTs `test`,
//! `save`, `enable`, `delete`. The settings (and the sealed key) live in the AI database; the source switch lives in the
//! main backend and is changed over the link.
//!
//! None of these routes touches the AI switch, the AI quota or the AI usage statistics. The key is never returned, never
//! logged and never put into an event; a provider's failure text has it replaced by `***`.
use super::{ok, only_keys, Answer, Ctx};
use crate::activity::Event;
use crate::codes::ApiError;
use crate::geo::mapping::{Mapping, FIELD_NAMES};
use crate::geo::template::{self, ProviderCfg};
use crate::geo::{self, Failure, FailureCode, GeoPlace, GEO_TEST_IP};
use crate::link::GeoSource;
use crate::secret::{self, GEO_KEY_AAD};
use crate::store::geo::{GeoRow, SealedKey, PRESETS};
use serde_json::{json, Map, Value};
use std::net::IpAddr;
use std::time::Instant;
use zeroize::Zeroizing;

/// What the draft says about the key.
enum KeyInput {
    /// `key` absent: keep the saved key
    Keep,
    /// `key: null`: no key
    Remove,
    /// `key: "..."`: this key
    Set(Zeroizing<String>),
}

struct Draft {
    preset: String,
    cfg: ProviderCfg,
    key: KeyInput,
}

fn str_of<'a>(body: &'a Map<String, Value>, name: &str) -> Result<&'a str, ApiError> {
    match body.get(name) {
        Some(Value::String(s)) => Ok(s),
        _ => Err(ApiError::bad_request()),
    }
}

/// Reads a draft body (3.3): the keys are exactly `preset`, `urlTemplate`, `keyHeaderName`, `keyHeaderValue`, `mapping`
/// and optionally `key`; the mapping has exactly its eight string keys. A wrong shape is 400 `ai.badRequest`.
fn parse_draft(body: &Map<String, Value>) -> Result<Draft, ApiError> {
    only_keys(
        body,
        &[
            "preset",
            "urlTemplate",
            "keyHeaderName",
            "keyHeaderValue",
            "key",
            "mapping",
        ],
    )?;
    let preset = str_of(body, "preset")?;
    if !PRESETS.contains(&preset) {
        return Err(ApiError::bad_request());
    }
    let Some(Value::Object(m)) = body.get("mapping") else {
        return Err(ApiError::bad_request());
    };
    if m.len() != FIELD_NAMES.len() {
        return Err(ApiError::bad_request());
    }
    let mut mapping = Mapping::default();
    for name in FIELD_NAMES {
        match m.get(name) {
            Some(Value::String(v)) => {
                mapping.set(name, v.clone());
            }
            _ => return Err(ApiError::bad_request()),
        }
    }
    let key = match body.get("key") {
        None => KeyInput::Keep,
        Some(Value::Null) => KeyInput::Remove,
        Some(Value::String(k)) => KeyInput::Set(Zeroizing::new(k.clone())),
        Some(_) => return Err(ApiError::bad_request()),
    };
    // A success value without a success path means nothing: it is dropped
    if mapping.ok_path.is_empty() {
        mapping.ok_value.clear();
    }
    Ok(Draft {
        preset: preset.to_string(),
        cfg: ProviderCfg {
            url_template: str_of(body, "urlTemplate")?.to_string(),
            key_header_name: str_of(body, "keyHeaderName")?.to_string(),
            key_header_value: str_of(body, "keyHeaderValue")?.to_string(),
            mapping,
        },
        key,
    })
}

/// The rules of 5.1 in their order: URL, header, key format, key needed, mapping. `saved` is the stored row (a kept key
/// counts as present).
fn validate(draft: &Draft, saved: Option<&GeoRow>) -> Result<(), ApiError> {
    let cfg = &draft.cfg;
    template::validate_url(&cfg.url_template)?;
    template::validate_header(&cfg.key_header_name, &cfg.key_header_value)?;
    let has_key = match &draft.key {
        KeyInput::Set(k) => {
            if !secret::valid_key(k) {
                return Err(ApiError::geo_bad_key());
            }
            true
        }
        KeyInput::Remove => false,
        KeyInput::Keep => saved.is_some_and(|r| r.key.is_some()),
    };
    if !has_key
        && template::needs_key(
            &cfg.url_template,
            &cfg.key_header_name,
            &cfg.key_header_value,
        )
    {
        return Err(ApiError::geo_key_required());
    }
    template::validate_mapping(&cfg.mapping)
}

fn settings_json(app: &crate::app::App, row: &GeoRow) -> Value {
    let unreadable = row
        .key
        .as_ref()
        .is_some_and(|k| geo::open_key(app, k).is_err());
    json!({
        "preset": row.preset,
        "urlTemplate": row.cfg.url_template,
        "keyHeaderName": row.cfg.key_header_name,
        "keyHeaderValue": row.cfg.key_header_value,
        "key": {
            "set": row.key.is_some(),
            "hint": row.key.as_ref().map_or("", |k| k.hint.as_str()),
            "unreadable": unreadable,
        },
        "mapping": row.cfg.mapping.to_json(),
        "host": template::host_of(&row.cfg.url_template),
        "insecureHttp": template::is_insecure_http(&row.cfg.url_template),
        "testedAt": row.tested_at,
        "updatedAt": row.updated_at,
    })
}

/// `GET /api/ai/admin/geo`
pub async fn get(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let row = app.geo.config();
    ok(json!({
        "settings": row.as_deref().map(|r| settings_json(app, r)),
        "testIp": GEO_TEST_IP,
        "perMinute": crate::app::Knobs::get(&app.knobs.geo_per_minute),
        "timeoutMs": u64::try_from(app.geo.total.as_millis()).unwrap_or(u64::MAX),
    }))
}

/// The result of one test lookup and how long it took.
struct Tested {
    result: Result<GeoPlace, Failure>,
    ms: i64,
}

impl Tested {
    fn code(&self) -> Option<&'static str> {
        self.result.as_ref().err().map(|f| f.code.as_str())
    }

    /// The fields that both the test and the save answers carry after `ok`.
    fn fill(&self, m: &mut Map<String, Value>) {
        match &self.result {
            Ok(place) => {
                m.insert("place".into(), place.to_json());
            }
            Err(f) => {
                m.insert("failure".into(), f.to_json());
            }
        }
        m.insert("ms".into(), Value::from(self.ms));
    }
}

/// Looks the example address up through the draft. A kept key that cannot be opened is a `keyProblem` without a call.
async fn run_test(ctx: &Ctx, draft: &Draft, saved: Option<&GeoRow>) -> Tested {
    let app = &ctx.app.0;
    let started = Instant::now();
    let key: Option<Zeroizing<String>> = match &draft.key {
        KeyInput::Set(k) => Some(k.clone()),
        KeyInput::Remove => None,
        KeyInput::Keep => match saved.map(|r| geo::saved_key(app, r)) {
            None => None,
            Some(Ok(k)) => k,
            Some(Err(_)) => {
                return Tested {
                    result: Err(Failure::code(FailureCode::KeyProblem)),
                    ms: 0,
                }
            }
        },
    };
    let ip: IpAddr = GEO_TEST_IP.parse().unwrap_or(IpAddr::from([8, 8, 8, 8]));
    let result = if geo::filter::is_public(ip) {
        geo::lookup_with(app, &draft.cfg, key.as_ref().map(|k| k.as_str()), ip).await
    } else {
        Err(Failure::code(FailureCode::Refused))
    };
    Tested {
        result,
        ms: i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
    }
}

fn test_event(ctx: &Ctx, action: &'static str, host: &str, t: &Tested, extra: Value) -> Event {
    let mut detail = match extra {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    detail.insert("host".into(), Value::from(host));
    let ev = Event::new(Some(&ctx.who), action, &ctx.client);
    match t.code() {
        None => ev.detail(Value::Object(detail)),
        Some(code) => {
            detail.insert("code".into(), Value::from(code));
            ev.detail(Value::Object(detail)).failed(code)
        }
    }
}

/// `POST /api/ai/admin/geo/test`: looks `GEO_TEST_IP` up through the draft; saves nothing.
pub async fn test(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let mut draft = parse_draft(&ctx.body)?;
    let saved = app.geo.config();
    // A saved key never follows the draft to another origin
    if matches!(draft.key, KeyInput::Keep)
        && saved.as_deref().is_some_and(|r| {
            r.key.is_some()
                && template::origin(&r.cfg.url_template)
                    != template::origin(&draft.cfg.url_template)
        })
    {
        draft.key = KeyInput::Remove;
    }
    validate(&draft, saved.as_deref())?;
    let _guard = app.geo.begin_test().ok_or_else(ApiError::geo_busy)?;
    let t = run_test(&ctx, &draft, saved.as_deref()).await;
    let host = template::host_of(&draft.cfg.url_template);
    app.activity(test_event(&ctx, "ai.geo.test", &host, &t, Value::Null));
    let mut m = Map::new();
    m.insert("ok".into(), Value::Bool(t.result.is_ok()));
    m.insert("testIp".into(), Value::from(GEO_TEST_IP));
    t.fill(&mut m);
    ok(Value::Object(m))
}

/// `POST /api/ai/admin/geo/save`: the same test first; on a pass the row is stored (key sealed, kept or removed).
pub async fn save(ctx: Ctx) -> Answer {
    let app = &ctx.app.0;
    let mut draft = parse_draft(&ctx.body)?;
    let saved = app.geo.config();
    // A saved key never follows the draft to another origin
    if matches!(draft.key, KeyInput::Keep)
        && saved.as_deref().is_some_and(|r| {
            r.key.is_some()
                && template::origin(&r.cfg.url_template)
                    != template::origin(&draft.cfg.url_template)
        })
    {
        draft.key = KeyInput::Remove;
    }
    validate(&draft, saved.as_deref())?;
    // A key can only be sealed with a working encryption key
    if matches!(draft.key, KeyInput::Set(_)) && app.master().is_none() {
        return Err(ApiError::geo_key_problem());
    }
    let _guard = app.geo.begin_test().ok_or_else(ApiError::geo_busy)?;
    let t = run_test(&ctx, &draft, saved.as_deref()).await;
    let host = template::host_of(&draft.cfg.url_template);
    let key_word = match &draft.key {
        KeyInput::Keep => "kept",
        KeyInput::Remove => "removed",
        KeyInput::Set(_) => "set",
    };
    let extra = json!({"change": draft.preset, "secret": key_word});
    app.activity(test_event(&ctx, "ai.geo.save", &host, &t, extra));
    let mut m = Map::new();
    if t.result.is_err() {
        m.insert("ok".into(), Value::Bool(false));
        t.fill(&mut m);
        return ok(Value::Object(m));
    }
    let now = crate::util::unix_now();
    let key: Option<SealedKey> = match &draft.key {
        KeyInput::Keep => saved.as_deref().and_then(|r| r.key.clone()),
        KeyInput::Remove => None,
        KeyInput::Set(k) => app.master().map(|master| {
            let sealed = master.seal_aad(GEO_KEY_AAD, k.as_bytes());
            SealedKey {
                key_id: master.key_id().to_string(),
                nonce: sealed.nonce.to_vec(),
                ciphertext: sealed.ciphertext,
                hint: secret::hint(k),
            }
        }),
    };
    let row = GeoRow {
        preset: draft.preset.clone(),
        cfg: draft.cfg.clone(),
        key,
        tested_at: now,
        updated_at: now,
        updated_by: Some(ctx.who.id),
    };
    app.store.write(|tx| crate::store::geo::save(tx, &row))?;
    app.reload_geo()?;
    m.insert("ok".into(), Value::Bool(true));
    m.insert("settings".into(), settings_json(app, &row));
    t.fill(&mut m);
    ok(Value::Object(m))
}

/// `POST /api/ai/admin/geo/enable`: switches the main backend to the external lookup. Needs saved (= tested) settings.
pub async fn enable(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &[])?;
    let app = &ctx.app.0;
    let Some(row) = app.geo.config() else {
        return Err(ApiError::geo_test_first());
    };
    if row
        .key
        .as_ref()
        .is_some_and(|k| geo::open_key(app, k).is_err())
    {
        return Err(ApiError::geo_key_problem());
    }
    match app.link.geo_source_set(GeoSource::Api).await {
        Ok(GeoSource::Api) => {}
        _ => return Err(ApiError::geo_link_down()),
    }
    app.activity(
        Event::new(Some(&ctx.who), "ai.geo.enable", &ctx.client)
            .detail(json!({"host": template::host_of(&row.cfg.url_template)})),
    );
    ok(json!({"source": "api"}))
}

/// `POST /api/ai/admin/geo/delete`: back to Cloudflare headers first, then the settings and the key are deleted.
pub async fn delete(ctx: Ctx) -> Answer {
    only_keys(&ctx.body, &[])?;
    let app = &ctx.app.0;
    match app.link.geo_source_set(GeoSource::Cloudflare).await {
        Ok(GeoSource::Cloudflare) => {}
        _ => return Err(ApiError::geo_link_down()),
    }
    app.store.write(|tx| crate::store::geo::delete(tx))?;
    app.reload_geo()?;
    app.activity(Event::new(Some(&ctx.who), "ai.geo.delete", &ctx.client));
    ok(json!({"settings": null, "source": "cloudflare"}))
}
