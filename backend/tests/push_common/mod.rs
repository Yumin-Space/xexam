//! Shared helpers of the v3.1 push tests (v31_push.rs, v31_saved.rs): an app on a real database with a clock the test moves,
//! users, devices, subscriptions with real key pairs, banks, the outbox rows, a fake web-push user agent that decrypts what
//! the backend encrypted, and the probes that tell whether the other v3.1 packages are merged.
//!
//! Only made-up data. The including test file declares `mod apic` (api_common) and `mod dbc` (common).
#![allow(dead_code)]

use crate::{apic, dbc};
use axum::body::Body;
use axum::Router;
use ring::{aead, agreement, hkdf, rand::SystemRandom};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use xexam_backend::api::internal::InternalToken;
use xexam_backend::api::{build_router, AppState};
use xexam_backend::auth::{self, Role};
use xexam_backend::database::store::PushJob;
use xexam_backend::database::{Clock, Store, UserChange};
use xexam_backend::models::{ImportTarget, Viewer};
use xexam_backend::push::vapid::Vapid;
use xexam_backend::push::{allow, ece};
use xexam_backend::services::RealEngine;

pub const HOST: &str = "127.0.0.1:8765";
pub const ORIGIN: &str = "http://127.0.0.1:8765";
pub const NGINX_HOST: &str = "backend:8765";
pub const TOKEN: &str = "fake-internal-token-for-tests-0123456789";
pub const PW: &str = "kitchen-pass-1";
pub const FCM: &str = "https://fcm.googleapis.com/fcm/send/";
pub const DAY: i64 = 86_400;
pub const HOUR: i64 = 3_600;

/// Unix seconds of a UTC civil time.
pub fn ts(y: i16, mo: i8, d: i8, h: i8, mi: i8) -> i64 {
    jiff::civil::date(y, mo, d)
        .at(h, mi, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .expect("zoned")
        .timestamp()
        .as_second()
}

/// A clock the test moves (the store reads it for every write and for `unix_now`).
pub struct TClock(pub AtomicI64);

impl Clock for TClock {
    fn now(&self) -> String {
        jiff::Timestamp::from_second(self.0.load(Ordering::SeqCst))
            .expect("timestamp")
            .strftime("%Y-%m-%d %H:%M:%S")
            .to_string()
    }
    fn unix(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// The Argon2 hash of [`PW`], made once (it is slow by design).
pub fn pw_hash() -> &'static str {
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| auth::hash_password(PW).expect("hash"))
}

/// Whether the push package (P4a) is built: key generation, a public-key check, the endpoint allow-list, the texts and the
/// quiet hours all work. Tests that need it say so and return early while the foundation stubs are still in place.
pub fn crypto_ready() -> bool {
    let Ok((key, _)) = Vapid::generate() else {
        return false;
    };
    let night = ts(2026, 6, 11, 2, 0);
    ece::check_public_key(key.public_key())
        && allow::endpoint_allowed("https://fcm.googleapis.com/fcm/send/abc")
        && xexam_backend::push::texts::render(&json!({"k": "test"}), "en").is_some()
        && xexam_backend::push::quiet::release_at(night, &jiff::tz::TimeZone::UTC)
            == ts(2026, 6, 11, 8, 0)
}

/// Prints why a test returns early.
pub fn skipped(why: &str) {
    eprintln!("SKIPPED (runs once the other v3.1 packages are merged): {why}");
}

// ---------------------------------------------------------------- a fake web-push user agent

/// The key pair and auth secret of one subscription, as a browser holds them.
pub struct Ua {
    private: agreement::EphemeralPrivateKey,
    pub public: [u8; 65],
    pub auth: [u8; 16],
}

pub fn new_ua() -> Ua {
    let rng = SystemRandom::new();
    let private =
        agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).expect("key");
    let mut public = [0u8; 65];
    public.copy_from_slice(private.compute_public_key().expect("public").as_ref());
    let mut auth = [0u8; 16];
    ring::rand::SecureRandom::fill(&rng, &mut auth).expect("auth");
    Ua {
        private,
        public,
        auth,
    }
}

struct Len(usize);
impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand(prk: &hkdf::Prk, info: &[u8], len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    prk.expand(&[info], Len(len))
        .expect("expand")
        .fill(&mut out)
        .expect("fill");
    out
}

/// Decrypts an RFC 8291 `aes128gcm` record the way a browser does; returns the plaintext.
pub fn decrypt(ua: Ua, record: &[u8]) -> Vec<u8> {
    assert!(record.len() > 16 + 4 + 1 + 65 + 16, "record too short");
    let salt = &record[..16];
    assert_eq!(u32::from_be_bytes(record[16..20].try_into().unwrap()), 4096);
    assert_eq!(record[20], 65);
    let as_public = &record[21..86];
    let cipher = &record[86..];
    let secret = agreement::agree_ephemeral(
        ua.private,
        &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, as_public),
        |s| s.to_vec(),
    )
    .expect("agree");
    let prk_key = hkdf::Salt::new(hkdf::HKDF_SHA256, &ua.auth).extract(&secret);
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(&ua.public);
    info.extend_from_slice(as_public);
    let ikm = expand(&prk_key, &info, 32);
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
    let cek = expand(&prk, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = expand(&prk, b"Content-Encoding: nonce\0", 12);
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).expect("key"));
    let mut buf = cipher.to_vec();
    let plain = key
        .open_in_place(
            aead::Nonce::try_assume_unique_for_key(&nonce).expect("nonce"),
            aead::Aad::empty(),
            &mut buf,
        )
        .expect("decrypt");
    let mut plain = plain.to_vec();
    assert_eq!(
        plain.pop(),
        Some(2),
        "the record must end with the 0x02 delimiter"
    );
    plain
}

// ---------------------------------------------------------------- the environment

/// One row of `push_outbox`.
#[derive(Debug, Clone)]
pub struct Out {
    pub id: i64,
    pub sub: i64,
    pub kind: String,
    pub topic: Option<String>,
    pub payload: Value,
    pub urgency: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub state: String,
    pub next_at: i64,
    pub tries: i64,
    pub status: Option<i64>,
    pub done_at: Option<i64>,
}

pub struct Env {
    pub dir: tempfile::TempDir,
    pub db: PathBuf,
    pub store: Arc<Store>,
    pub state: AppState,
    pub router: Router,
    pub clock: Arc<TClock>,
    /// The text form of the test VAPID key (`None` while the crypto is not built)
    pub vapid_text: Option<String>,
    counter: AtomicUsize,
}

impl Env {
    /// An app whose clock starts at the real time (sessions made by sign-in stay valid).
    pub fn new() -> Env {
        Env::at(auth::unix_now())
    }

    /// An app whose clock starts at `t`. The administrator `admin` (id 1) and the made-up users `kim` (2) and `lee` (3) all
    /// have the password [`PW`] and no pending change. The test VAPID key is installed when the crypto exists.
    pub fn at(t: i64) -> Env {
        Env::at_with(t, &[])
    }

    pub fn at_with(t: i64, env: &[(&str, &str)]) -> Env {
        let dir = dbc::tmp();
        let db = dir.path().join("xexam.db");
        let clock = Arc::new(TClock(AtomicI64::new(t)));
        let store = Arc::new(Store::open_with_clock(&db, clock.clone()).expect("store"));
        let engine = Arc::new(RealEngine::new(store.clone()));
        let data = dir.path().to_string_lossy().to_string();
        let mut vars: Vec<(&str, &str)> = vec![("XEXAM_DATA", data.as_str())];
        vars.extend_from_slice(env);
        let state = AppState::new_with_logs(
            apic::config_with(&vars),
            store.clone(),
            engine,
            None,
            Default::default(),
        );
        assert!(state
            .0
            .ai_link
            .set_token(InternalToken::new(TOKEN).expect("token")));
        for name in ["kim", "lee"] {
            match store
                .create_user(name, pw_hash(), Role::User)
                .expect("user")
            {
                UserChange::Done(_) => {}
                other => panic!("{other:?}"),
            }
        }
        let e = Env {
            router: build_router(state.clone()),
            dir,
            db,
            store,
            state,
            clock,
            vapid_text: None,
            counter: AtomicUsize::new(0),
        };
        e.exec(
            "UPDATE users SET password_hash = ?1, must_change_password = 0, display_name = 'User ' || id",
            params![pw_hash()],
        );
        let mut e = e;
        if let Ok((_, text)) = Vapid::generate() {
            e.state.0.push.set_key(
                Some(Vapid::from_text(&text).expect("key")),
                "mailto:admin@xexam.example.com".to_string(),
            );
            e.vapid_text = Some(text);
        }
        e
    }

    pub fn raw(&self) -> Connection {
        Connection::open(&self.db).expect("raw open")
    }

    pub fn exec(&self, sql: &str, p: impl rusqlite::Params) {
        self.raw().execute(sql, p).expect(sql);
    }

    pub fn int(&self, sql: &str) -> i64 {
        self.raw().query_row(sql, [], |r| r.get(0)).expect(sql)
    }

    pub fn opt_int(&self, sql: &str) -> Option<i64> {
        self.raw().query_row(sql, [], |r| r.get(0)).expect(sql)
    }

    pub fn text(&self, sql: &str) -> String {
        self.raw().query_row(sql, [], |r| r.get(0)).expect(sql)
    }

    pub fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }

    pub fn set_now(&self, t: i64) {
        self.clock.0.store(t, Ordering::SeqCst);
    }

    pub fn advance(&self, secs: i64) {
        self.clock.0.fetch_add(secs, Ordering::SeqCst);
    }

    /// The installed test key (a second `Vapid` of the same text; `None` while the crypto is not built).
    pub fn key(&self) -> Option<Vapid> {
        self.vapid_text
            .as_deref()
            .map(|t| Vapid::from_text(t).expect("key"))
    }

    pub fn key_id(&self) -> String {
        self.key()
            .map(|k| k.key_id())
            .unwrap_or_else(|| "0123456789abcdef".to_string())
    }

    fn next(&self) -> usize {
        self.counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn user(&self, name: &str) -> i64 {
        self.int(&format!(
            "SELECT id FROM users WHERE username = '{name}' COLLATE NOCASE"
        ))
    }

    pub fn new_user(&self, name: &str, role: Role) -> i64 {
        match self.store.create_user(name, pw_hash(), role).expect("user") {
            UserChange::Done(id) => {
                self.exec(
                    "UPDATE users SET must_change_password = 0, display_name = 'User ' || id WHERE id = ?1",
                    params![id],
                );
                id
            }
            other => panic!("{other:?}"),
        }
    }

    pub fn set_setting(&self, user: i64, key: &str, json_text: &str) {
        self.exec(
            "INSERT INTO user_settings(user_id, key, value) VALUES(?1, ?2, ?3) \
             ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value",
            params![user, key, json_text],
        );
    }

    /// A device of `user` (zone `UTC` unless given), used just now.
    pub fn device(&self, user: i64, tz: Option<&str>) -> i64 {
        let n = self.next();
        let now = self.now();
        self.exec(
            "INSERT INTO devices(user_id, cookie_hash, created_at, active_at, tz) VALUES(?1, ?2, ?3, ?3, ?4)",
            params![user, auth::token_hash(&format!("cookie-{n}")), now, tz],
        );
        self.int("SELECT MAX(id) FROM devices")
    }

    /// A subscription for `device` with a fresh key pair and a unique allowed endpoint; `key_id` defaults to the test key's.
    pub fn sub(&self, device: i64) -> (i64, Ua) {
        self.sub_with(device, &self.key_id())
    }

    pub fn sub_with(&self, device: i64, key_id: &str) -> (i64, Ua) {
        let ua = new_ua();
        let n = self.next();
        let now = self.now();
        self.exec(
            "INSERT INTO push_subscriptions(device_id, endpoint, p256dh, auth, key_id, created_at, confirmed_at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                device,
                format!("{FCM}endpoint-{n}"),
                xexam_backend::security::base64url_nopad(&ua.public),
                xexam_backend::security::base64url_nopad(&ua.auth),
                key_id,
                now
            ],
        );
        (self.int("SELECT MAX(id) FROM push_subscriptions"), ua)
    }

    /// Every outbox row, oldest first.
    pub fn outbox(&self) -> Vec<Out> {
        let c = self.raw();
        let mut st = c
            .prepare(
                "SELECT id, subscription_id, kind, topic, payload, urgency, created_at, expires_at, state, next_at, tries, status, done_at \
                 FROM push_outbox ORDER BY id",
            )
            .expect("prepare");
        let rows = st
            .query_map([], |r| {
                let payload: String = r.get(4)?;
                Ok(Out {
                    id: r.get(0)?,
                    sub: r.get(1)?,
                    kind: r.get(2)?,
                    topic: r.get(3)?,
                    payload: serde_json::from_str(&payload).expect("payload json"),
                    urgency: r.get(5)?,
                    created_at: r.get(6)?,
                    expires_at: r.get(7)?,
                    state: r.get(8)?,
                    next_at: r.get(9)?,
                    tries: r.get(10)?,
                    status: r.get(11)?,
                    done_at: r.get(12)?,
                })
            })
            .expect("query");
        rows.map(|r| r.expect("row")).collect()
    }

    pub fn outbox_of(&self, sub: i64) -> Vec<Out> {
        self.outbox().into_iter().filter(|o| o.sub == sub).collect()
    }

    pub fn out(&self, id: i64) -> Out {
        self.outbox()
            .into_iter()
            .find(|o| o.id == id)
            .expect("outbox row")
    }

    /// Queues a row by hand (state `queued`), for the lease and result tests.
    pub fn raw_out(
        &self,
        sub: i64,
        kind: &str,
        topic: Option<&str>,
        payload: &Value,
        next_at: i64,
        ttl: i64,
    ) -> i64 {
        let now = self.now();
        self.exec(
            "INSERT INTO push_outbox(subscription_id, kind, topic, payload, urgency, created_at, expires_at, state, next_at, tries) \
             VALUES(?1, ?2, ?3, ?4, 'normal', ?5, ?6, 'queued', ?7, 0)",
            params![sub, kind, topic, payload.to_string(), now, now.max(next_at) + ttl, next_at],
        );
        self.int("SELECT MAX(id) FROM push_outbox")
    }

    /// Leases at the clock's time with the test key.
    pub fn lease(&self) -> Vec<PushJob> {
        self.lease_at(self.now())
    }

    pub fn lease_at(&self, now: i64) -> Vec<PushJob> {
        let key = self.key().expect("crypto");
        self.store
            .push_lease(now, 20, &key, "mailto:admin@xexam.example.com")
            .expect("lease")
    }

    // ------------------------------------------------------------ banks

    /// A new bank of `n` made-up questions through the store's import (what the HTTP import and the command line call);
    /// `viewer` = the signed-in importer, `None` = the command-line import. Returns the bank id.
    pub fn import_new(&self, viewer: Option<&Viewer>, file: &str, n: usize) -> i64 {
        let (sheets, qs) = bank_parts(file, n);
        let r = match viewer {
            Some(v) => self
                .store
                .apply_import_as(v, ImportTarget::New, file, &sheets, &qs),
            None => self
                .store
                .apply_import_for(ImportTarget::New, file, &sheets, &qs),
        };
        r.expect("import").expect("bank").1
    }

    /// Updates a bank so it holds questions `0..to` of `file` (the earlier ones stay, the new ones are added).
    pub fn import_more(&self, viewer: Option<&Viewer>, bank: i64, file: &str, to: usize) {
        let (sheets, qs) = bank_parts(file, to);
        let r = match viewer {
            Some(v) => self
                .store
                .apply_import_as(v, ImportTarget::Bank(bank), file, &sheets, &qs),
            None => self
                .store
                .apply_import_for(ImportTarget::Bank(bank), file, &sheets, &qs),
        };
        r.expect("update").expect("bank");
    }

    /// A personal AI-generated bank (`ai_job` set) held by `holders` through `user_banks`.
    pub fn ai_bank(&self, name: &str, owner: i64, holders: &[i64]) -> i64 {
        let now = self.store_now();
        self.exec(
            "INSERT INTO banks(name, filename, legacy_ids, created_at, updated_at, owner_id, ai_job, ai_meta) \
             VALUES(?1, ?2, 0, ?3, ?3, ?4, ?5, '{}')",
            params![name, format!("{name}.xlsx"), now, owner, format!("job-{name}")],
        );
        let bank = self.int("SELECT MAX(id) FROM banks");
        self.exec(
            "INSERT INTO questions(id, sheet, row_no, seq, data, active, added_at, updated_at, bank_id) \
             VALUES(?1, 'Generated', 1, 1, '{\"kind\": \"single\"}', 1, ?2, ?2, ?3)",
            params![format!("ai-{bank}-1"), now, bank],
        );
        for h in holders {
            self.exec(
                "INSERT INTO user_banks(user_id, bank_id) VALUES(?1, ?2)",
                params![h, bank],
            );
        }
        bank
    }

    fn store_now(&self) -> String {
        self.clock.now()
    }

    pub fn grant(&self, user: i64, bank: i64) {
        self.exec(
            "INSERT OR IGNORE INTO user_banks(user_id, bank_id) VALUES(?1, ?2)",
            params![user, bank],
        );
    }

    pub fn viewer(&self, user: i64) -> Viewer {
        self.store.viewer(user).expect("viewer")
    }

    /// Sets the "already announced" marks to the current ids and returns, so the next tick starts from here.
    pub fn mark_banks_seen(&self) {
        let banks = self.int("SELECT COALESCE(MAX(id), 0) FROM banks");
        let imports = self.int("SELECT COALESCE(MAX(id), 0) FROM imports");
        for (k, v) in [("push_banks_seen", banks), ("push_imports_seen", imports)] {
            self.exec(
                "INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, v.to_string()],
            );
        }
    }

    // ------------------------------------------------------------ HTTP

    pub async fn call(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: Option<String>,
    ) -> apic::Resp {
        let mut hs: Vec<(&str, Vec<u8>)> = Vec::new();
        if let Some(c) = cookie {
            hs.push(("cookie", c.as_bytes().to_vec()));
        }
        if method == "POST" {
            hs.push(("origin", ORIGIN.as_bytes().to_vec()));
        }
        let body = match body {
            Some(b) => {
                hs.push(("content-type", b"application/json".to_vec()));
                hs.push(("content-length", b.len().to_string().into_bytes()));
                Body::from(b)
            }
            None => Body::empty(),
        };
        let hrefs: Vec<(&str, &[u8])> = hs.iter().map(|(k, v)| (*k, v.as_slice())).collect();
        apic::send(
            &self.router,
            apic::build(method, path, Some(HOST), &hrefs, body),
        )
        .await
    }

    pub async fn get(&self, path: &str, cookie: &str) -> apic::Resp {
        self.call("GET", path, Some(cookie), None).await
    }

    pub async fn post(&self, path: &str, cookie: &str, body: Value) -> apic::Resp {
        self.call("POST", path, Some(cookie), Some(body.to_string()))
            .await
    }

    /// Signs `name` in through the API; returns the `xexam_session=<token>` cookie pair.
    pub async fn login(&self, name: &str) -> String {
        let r = self
            .call(
                "POST",
                "/api/auth/login",
                None,
                Some(json!({"username": name, "password": PW}).to_string()),
            )
            .await;
        assert_eq!(r.status, 200, "{}", r.text());
        r.header("set-cookie")
            .expect("set-cookie")
            .split(';')
            .next()
            .expect("pair")
            .to_string()
    }

    /// Gives the session of `cookie` the device `device` (what the devices package does at sign-in).
    pub fn link_session(&self, cookie: &str, device: i64) {
        self.exec(
            "UPDATE sessions SET device_id = ?1 WHERE token_hash = ?2",
            params![
                device,
                auth::token_hash(cookie.trim_start_matches("xexam_session="))
            ],
        );
    }

    /// Whether the devices package is merged: a session linked to a device reaches the handlers as `Authed.device_id`.
    pub async fn device_linking_works(&self) -> bool {
        let user = self.user("lee");
        let cookie = self.login("lee").await;
        let device = self.device(user, Some("UTC"));
        self.link_session(&cookie, device);
        let r = self
            .post(
                "/api/study/saved/discard",
                &cookie,
                json!({"key": "00000000000000000000000000000000"}),
            )
            .await;
        // discard needs no device; the saved report does
        assert_eq!(r.status, 200, "{}", r.text());
        let r = self
            .post(
                "/api/study/saved",
                &cookie,
                json!({"key": "00000000000000000000000000000000", "kind": "practice", "answered": 0, "total": 1, "startedAt": 1_780_000_000, "timeLeft": null}),
            )
            .await;
        let ok = r.status == 200;
        self.exec("DELETE FROM saved_sessions", []);
        self.exec("DELETE FROM sessions WHERE device_id = ?1", params![device]);
        self.exec("DELETE FROM devices WHERE id = ?1", params![device]);
        ok
    }

    /// A token route as the AI service calls it.
    pub async fn internal(&self, path: &str, token: Option<&str>, body: &str) -> apic::Resp {
        let len = body.len().to_string();
        let mut hs: Vec<(&str, &[u8])> = vec![
            ("content-type", b"application/json"),
            ("content-length", len.as_bytes()),
        ];
        if let Some(t) = token {
            hs.push(("x-xexam-internal-token", t.as_bytes()));
        }
        apic::send(
            &self.router,
            apic::build(
                "POST",
                path,
                Some(NGINX_HOST),
                &hs,
                Body::from(body.to_string()),
            ),
        )
        .await
    }

    /// The activity rows of `action` (flushed first): `(username, detail)`.
    pub async fn entries(&self, action: &str) -> Vec<(Option<String>, Option<String>)> {
        self.state.0.activity.flush().await;
        let c = self.raw();
        let mut st = c
            .prepare("SELECT username, detail FROM activity_log WHERE action = ?1 ORDER BY id")
            .expect("prepare");
        let rows = st
            .query_map([action], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("query");
        rows.map(|r| r.expect("row")).collect()
    }
}

/// The sheet and questions of a made-up bank: questions `from..to` of `file`.
fn bank_parts(
    file: &str,
    to: usize,
) -> (
    Vec<xexam_backend::models::ParsedSheet>,
    Vec<xexam_backend::models::Question>,
) {
    let qs = (0..to)
        .map(|i| {
            dbc::single(
                file,
                &format!("{file}-{i}"),
                &format!("{file} made-up question {i}?"),
                i as i64 + 1,
            )
        })
        .collect();
    (vec![dbc::sheet(file, to as i64)], qs)
}
