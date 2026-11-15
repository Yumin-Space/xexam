//! Model routing (v1.3): which connection and model a call goes to.
//!
//! One function resolves it for every caller: the administrator's chat pick, then the slot's model, then the default
//! slot's model, else `ai.disabled`. There is no automatic switch to any other model (owner: a warning only). A model is
//! usable when its status is `ok`, its connection is not `broken`, and the keys the connection's provider needs are
//! saved and readable with the current encryption key. The resolver itself is pure (`View::resolve`), so the rules are
//! tested without a database.
use crate::codes::ApiError;
use crate::effort::Effort;
use crate::settings::{Format, Provider};
use crate::store::conn::{ConnRow, PoolRow, SLOT_NAMES};
use serde_json::{json, Value};
use std::collections::HashMap;

/// The wire format of a route's requests (`settings::Format`).
pub type WireFormat = Format;

/// The six model slots. `Default` is the model every other slot falls back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Default,
    Chat,
    Study,
    Summary,
    Generate,
    Import,
}

impl Slot {
    /// Every slot, in the order of `store::conn::SLOT_NAMES`.
    pub const ALL: [Slot; 6] = [
        Slot::Default,
        Slot::Chat,
        Slot::Study,
        Slot::Summary,
        Slot::Generate,
        Slot::Import,
    ];

    /// `default | chat | study | summary | generate | import`
    pub fn as_str(self) -> &'static str {
        match self {
            Slot::Default => "default",
            Slot::Chat => "chat",
            Slot::Study => "study",
            Slot::Summary => "summary",
            Slot::Generate => "generate",
            Slot::Import => "import",
        }
    }

    pub fn parse(s: &str) -> Option<Slot> {
        Slot::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// Position in [`Slot::ALL`] / `SLOT_NAMES` / the array `store::conn::slots` returns.
    pub fn index(self) -> usize {
        match self {
            Slot::Default => 0,
            Slot::Chat => 1,
            Slot::Study => 2,
            Slot::Summary => 3,
            Slot::Generate => 4,
            Slot::Import => 5,
        }
    }
}

impl crate::features::Feature {
    /// The model slot a feature uses: hint and explanation share `study`.
    pub fn slot(self) -> Slot {
        use crate::features::Feature;
        match self {
            Feature::Chat => Slot::Chat,
            Feature::Hint | Feature::Explain => Slot::Study,
            Feature::Summary => Slot::Summary,
            Feature::Generate => Slot::Generate,
            Feature::Import => Slot::Import,
        }
    }
}

/// An administrator's own choice of connection and model (the AI Chat model menu), outside the slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub connection_id: i64,
    pub model: String,
}

/// A model id is 1..200 characters, with no control character and no whitespace at either end.
pub fn model_id_ok(s: &str) -> bool {
    let n = s.chars().count();
    (1..=200).contains(&n) && !s.chars().any(char::is_control) && s.trim() == s
}

impl Pick {
    /// `null` is the feature's default (`None`); anything that is not exactly `{"connectionId", "model"}` with a valid
    /// connection id and model id is a 400 `ai.badModel`.
    pub fn from_json(v: &Value) -> Result<Option<Pick>, ApiError> {
        let obj = match v {
            Value::Null => return Ok(None),
            Value::Object(o) => o,
            _ => return Err(ApiError::bad_model()),
        };
        if obj.len() != 2 {
            return Err(ApiError::bad_model());
        }
        let id = obj
            .get("connectionId")
            .and_then(Value::as_i64)
            .filter(|n| *n > 0)
            .ok_or_else(ApiError::bad_model)?;
        let model = obj
            .get("model")
            .and_then(Value::as_str)
            .filter(|m| model_id_ok(m))
            .ok_or_else(ApiError::bad_model)?;
        Ok(Some(Pick {
            connection_id: id,
            model: model.to_string(),
        }))
    }

    /// `"<connectionId>:<model id>"`, split at the FIRST colon (model ids may contain colons).
    pub fn parse_text(s: &str) -> Option<Pick> {
        let (id, model) = s.split_once(':')?;
        if !(1..=18).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let connection_id = id.parse::<i64>().ok().filter(|n| *n > 0)?;
        model_id_ok(model).then(|| Pick {
            connection_id,
            model: model.to_string(),
        })
    }

    pub fn to_json(&self) -> Value {
        json!({"connectionId": self.connection_id, "model": self.model})
    }

    /// The text form, the inverse of [`Pick::parse_text`].
    pub fn to_text(&self) -> String {
        format!("{}:{}", self.connection_id, self.model)
    }
}

/// Which model a route came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// The pick, or the slot's own model (for the default slot: the default model)
    None,
    /// The slot has no usable model of its own; the default slot's model is used
    Default,
}

impl Fallback {
    pub fn as_str(self) -> &'static str {
        match self {
            Fallback::None => "none",
            Fallback::Default => "default",
        }
    }
}

/// Where one call goes. No key, no URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub connection_id: i64,
    pub connection_name: String,
    pub provider: Provider,
    pub format: WireFormat,
    pub model: String,
    /// `None` for an administrator's pick outside the pool
    pub pool_id: Option<i64>,
    /// 1 = send `stream_options.include_usage` (OpenAI format)
    pub usage_option: i64,
    /// 4096 or 8192: the ladder ceiling of the model
    pub max_tokens: i64,
    /// Context window in tokens when the listing named one; callers fall back to 128 000
    pub context: Option<i64>,
    /// `"c<id>r<rev>:<model>"`: the key of the in-memory memos (token ladder, questions per call)
    pub key: String,
    pub fallback: Fallback,
    /// An administrator's pick was given but did not resolve (the chat sets `start.modelFallback`)
    pub pick_dropped: bool,
    /// The thinking strength of the REQUESTED slot (not of the slot whose model is used); `None` = send nothing
    pub effort: Option<Effort>,
}

/// What the resolver needs to know about the keys of one connection (never key material).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyState {
    pub api: bool,
    pub gateway: bool,
    /// Every saved key was sealed with the current encryption key
    pub readable: bool,
}

/// The routing tables as read at one moment.
#[derive(Debug, Clone, Default)]
pub struct View {
    pub conns: Vec<ConnRow>,
    pub pool: Vec<PoolRow>,
    /// Pool id of each slot, in [`Slot::ALL`] order
    pub slots: [Option<i64>; 6],
    /// Thinking strength of each slot, in [`Slot::ALL`] order
    pub efforts: [Option<Effort>; 6],
    pub keys: HashMap<i64, KeyState>,
}

impl View {
    pub fn conn(&self, id: i64) -> Option<&ConnRow> {
        self.conns.iter().find(|c| c.id == id)
    }

    fn pool_row(&self, id: i64) -> Option<&PoolRow> {
        self.pool.iter().find(|m| m.id == id)
    }

    /// The connection can serve calls: not broken, and the keys its provider needs are saved and readable.
    pub fn conn_usable(&self, c: &ConnRow) -> bool {
        if c.status == "broken" {
            return false;
        }
        let Some(provider) = Provider::parse(&c.provider) else {
            return false;
        };
        let k = self.keys.get(&c.id).copied().unwrap_or_default();
        let saved = match provider.required_key() {
            None => true,
            Some(crate::secret::SecretName::ApiKey) => k.api,
            Some(crate::secret::SecretName::GatewayKey) => k.gateway,
        };
        saved && k.readable
    }

    /// A pool model that can serve calls: status `ok` and a usable connection.
    pub fn model_usable(&self, m: &PoolRow) -> bool {
        m.status == "ok"
            && self
                .conn(m.connection_id)
                .is_some_and(|c| self.conn_usable(c))
    }

    fn route_of(
        &self,
        c: &ConnRow,
        model: &str,
        row: Option<&PoolRow>,
        fallback: Fallback,
    ) -> Option<Route> {
        let provider = Provider::parse(&c.provider)?;
        let format = match provider {
            Provider::OpenAi => Format::OpenAi,
            Provider::Anthropic => Format::Anthropic,
            Provider::Gateway => Format::parse(&c.format)?,
        };
        Some(Route {
            connection_id: c.id,
            connection_name: c.name.clone(),
            provider,
            format,
            model: model.to_string(),
            pool_id: row.map(|m| m.id),
            usage_option: row.map_or(1, |m| i64::from(m.usage_option)),
            max_tokens: row.map_or(8192, |m| m.max_tokens),
            context: row.and_then(|m| m.context),
            key: format!("c{}r{}:{}", c.id, c.rev, model),
            fallback,
            pick_dropped: false,
            effort: None,
        })
    }

    /// The route of a slot's own pool model, when it is set and usable.
    fn slot_route(&self, slot: Slot, fallback: Fallback) -> Option<Route> {
        let id = (*self.slots.get(slot.index())?)?;
        let m = self.pool_row(id)?;
        if !self.model_usable(m) {
            return None;
        }
        let c = self.conn(m.connection_id)?;
        self.route_of(c, &m.model, Some(m), fallback)
    }

    /// The pick of an administrator: a usable connection and any model of it (a model outside the pool is allowed).
    fn pick_route(&self, p: &Pick) -> Option<Route> {
        let c = self.conn(p.connection_id).filter(|c| self.conn_usable(c))?;
        let row = self
            .pool
            .iter()
            .find(|m| m.connection_id == c.id && m.model == p.model);
        self.route_of(c, &p.model, row, Fallback::None)
    }

    /// pick -> the slot's model -> the default slot's model -> `ai.disabled`. Nothing else, ever.
    pub fn resolve(&self, slot: Slot, pick: Option<&Pick>) -> Result<Route, ApiError> {
        let effort = self.efforts[slot.index()];
        let mut dropped = false;
        if let Some(p) = pick {
            if let Some(mut r) = self.pick_route(p) {
                r.effort = effort;
                return Ok(r);
            }
            dropped = true;
        }
        let found = self.slot_route(slot, Fallback::None).or_else(|| {
            (slot != Slot::Default)
                .then(|| self.slot_route(Slot::Default, Fallback::Default))
                .flatten()
        });
        match found {
            Some(mut r) => {
                r.pick_dropped = dropped;
                r.effort = effort;
                Ok(r)
            }
            None => Err(ApiError::disabled()),
        }
    }

    /// `true` when `slot` resolves without a pick.
    pub fn exists(&self, slot: Slot) -> bool {
        self.resolve(slot, None).is_ok()
    }

    /// `true` when at least one of the six slots resolves.
    pub fn any(&self) -> bool {
        Slot::ALL.iter().any(|s| self.exists(*s))
    }
}

/// The slot names in the store's order (kept next to [`Slot::ALL`] so the two cannot drift apart).
pub fn slot_names_match() -> bool {
    Slot::ALL
        .iter()
        .zip(SLOT_NAMES)
        .all(|(s, n)| s.as_str() == n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: i64, provider: &str, status: &str) -> ConnRow {
        ConnRow {
            id,
            rev: 3,
            name: format!("conn{id}"),
            provider: provider.into(),
            base_url: "https://api.example.com/v1".into(),
            format: "openai".into(),
            gateway_auth: "bearer".into(),
            status: status.into(),
            error_code: None,
            lists_models: true,
            tested_at: None,
            sort: id,
            created_at: 0,
            updated_at: 0,
            updated_by: String::new(),
        }
    }

    fn model(id: i64, conn: i64, name: &str, status: &str) -> PoolRow {
        PoolRow {
            id,
            connection_id: conn,
            model: name.into(),
            manual: false,
            status: status.into(),
            error_code: None,
            usage_option: true,
            max_tokens: 4096,
            context: Some(64_000),
            latency_ms: None,
            tested_at: None,
        }
    }

    fn keys(api: bool, gateway: bool, readable: bool) -> KeyState {
        KeyState {
            api,
            gateway,
            readable,
        }
    }

    /// Connection 1 (openai, no key needed) with models 10 (small) and 11 (large); connection 2 (anthropic) with 20.
    fn view() -> View {
        let mut v = View {
            conns: vec![conn(1, "openai", "ok"), conn(2, "anthropic", "ok")],
            pool: vec![
                model(10, 1, "demo-small", "ok"),
                model(11, 1, "demo-large", "ok"),
                model(20, 2, "demo-claude", "ok"),
            ],
            slots: [Some(10), None, Some(20), None, None, None],
            efforts: [None; 6],
            keys: HashMap::new(),
        };
        v.keys.insert(1, keys(false, false, true));
        v.keys.insert(2, keys(true, false, true));
        v
    }

    #[test]
    fn resolve_carries_requested_slot_effort() {
        use crate::effort::{Effort, EffortFamily};
        let hi = Effort::new(EffortFamily::OpenAi, "high");
        let lo = Effort::new(EffortFamily::Anthropic, "low");
        let md = Effort::new(EffortFamily::Google, "medium");
        let mut v = view();
        // default: md, chat: hi (falls back to the default model), study: lo (own model), summary: none
        v.efforts = [md, hi, lo, None, None, None];
        // own model: the slot's effort
        let r = v.resolve(Slot::Study, None).expect("study");
        assert_eq!((r.fallback, r.effort), (Fallback::None, lo));
        // Default fallback: the requested slot's effort, not Default's
        let r = v.resolve(Slot::Chat, None).expect("chat");
        assert_eq!((r.fallback, r.effort), (Fallback::Default, hi));
        // no effort row: nothing, although the Default slot has one
        let r = v.resolve(Slot::Summary, None).expect("summary");
        assert_eq!(r.effort, None);
        // the Default slot itself
        assert_eq!(v.resolve(Slot::Default, None).expect("d").effort, md);
        // an administrator's pick carries the Chat effort
        let pick = Pick {
            connection_id: 2,
            model: "demo-claude".into(),
        };
        let r = v.resolve(Slot::Chat, Some(&pick)).expect("pick");
        assert_eq!(r.effort, hi);
        // a dropped pick: still the slot's effort
        let bad = Pick {
            connection_id: 99,
            model: "x".into(),
        };
        let r = v.resolve(Slot::Chat, Some(&bad)).expect("dropped");
        assert!(r.pick_dropped);
        assert_eq!(r.effort, hi);
    }

    #[test]
    fn slot_names_and_features() {
        assert!(slot_names_match());
        for (i, s) in Slot::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
            assert_eq!(Slot::parse(s.as_str()), Some(*s));
        }
        assert_eq!(Slot::parse("Default"), None);
        use crate::features::Feature;
        assert_eq!(Feature::Hint.slot(), Slot::Study);
        assert_eq!(Feature::Explain.slot(), Slot::Study);
        assert_eq!(Feature::Chat.slot(), Slot::Chat);
        assert_eq!(Feature::Summary.slot(), Slot::Summary);
        assert_eq!(Feature::Generate.slot(), Slot::Generate);
        assert_eq!(Feature::Import.slot(), Slot::Import);
    }

    #[test]
    fn slot_own_model_then_default_then_nothing() {
        let v = view();
        // study has its own model
        let r = v.resolve(Slot::Study, None).expect("study");
        assert_eq!(
            (r.model.as_str(), r.fallback),
            ("demo-claude", Fallback::None)
        );
        assert_eq!(r.format, Format::Anthropic);
        assert_eq!(r.key, "c2r3:demo-claude");
        assert_eq!(r.pool_id, Some(20));
        assert_eq!(
            (r.usage_option, r.max_tokens, r.context),
            (1, 4096, Some(64_000))
        );
        // chat has none: the default model, marked as a fallback
        let r = v.resolve(Slot::Chat, None).expect("chat");
        assert_eq!(
            (r.model.as_str(), r.fallback),
            ("demo-small", Fallback::Default)
        );
        // the default slot itself is not a fallback
        let r = v.resolve(Slot::Default, None).expect("default");
        assert_eq!(r.fallback, Fallback::None);
        // no default and no own model: nothing resolves, and other pool models are never picked up
        let mut none = view();
        none.slots = [None; 6];
        for s in Slot::ALL {
            assert_eq!(
                none.resolve(s, None).err().map(|e| e.code),
                Some("ai.disabled")
            );
        }
        assert!(!none.any());
    }

    #[test]
    fn never_another_pool_model() {
        // the default model is unusable and the chat slot is empty: no route, although model 11 is fine
        let mut v = view();
        v.pool[0].status = "failed".into();
        assert!(v.resolve(Slot::Chat, None).is_err());
        assert!(!v.exists(Slot::Default));
        // study still has its own model: AI is not "no route" as a whole
        assert!(v.exists(Slot::Study));
        assert!(v.any());
        // a feature whose own model is unusable falls back to the default when that one is fine
        let mut v = view();
        v.pool[2].status = "untested".into();
        let r = v.resolve(Slot::Study, None).expect("study");
        assert_eq!(
            (r.model.as_str(), r.fallback),
            ("demo-small", Fallback::Default)
        );
    }

    #[test]
    fn broken_connection_and_keys_make_a_model_unusable() {
        let mut v = view();
        v.conns[0].status = "broken".into();
        assert!(!v.exists(Slot::Default));
        // a provider that needs a key without one saved
        let mut v = view();
        v.keys.insert(2, keys(false, false, true));
        let r = v.resolve(Slot::Study, None).expect("falls back to default");
        assert_eq!(r.fallback, Fallback::Default);
        // keys sealed with another encryption key
        let mut v = view();
        v.keys.insert(2, keys(true, false, false));
        assert_eq!(
            v.resolve(Slot::Study, None).expect("study").connection_id,
            1
        );
        // a gateway needs the gateway key, not the provider key
        let mut v = view();
        v.conns.push(conn(3, "gateway", "ok"));
        v.pool.push(model(30, 3, "demo-gw", "ok"));
        v.slots[1] = Some(30);
        v.keys.insert(3, keys(true, false, true));
        assert_eq!(v.resolve(Slot::Chat, None).expect("chat").connection_id, 1);
        v.keys.insert(3, keys(false, true, true));
        let r = v.resolve(Slot::Chat, None).expect("chat");
        assert_eq!((r.connection_id, r.fallback), (3, Fallback::None));
    }

    #[test]
    fn a_pick_wins_and_may_leave_the_pool() {
        let v = view();
        let p = Pick {
            connection_id: 1,
            model: "demo-large".into(),
        };
        let r = v.resolve(Slot::Chat, Some(&p)).expect("pick");
        assert_eq!(
            (r.model.as_str(), r.pool_id, r.pick_dropped),
            ("demo-large", Some(11), false)
        );
        // a model that is not in the pool: allowed for a pick, with the defaults of the picker
        let off = Pick {
            connection_id: 2,
            model: "not-in-pool".into(),
        };
        let r = v.resolve(Slot::Chat, Some(&off)).expect("off-pool pick");
        assert_eq!(r.pool_id, None);
        assert_eq!((r.usage_option, r.max_tokens, r.context), (1, 8192, None));
        assert_eq!(r.key, "c2r3:not-in-pool");
        // a pick of a broken or missing connection is dropped, the slot's model is used instead
        let mut b = view();
        b.conns[0].status = "broken".into();
        let r = b.resolve(Slot::Study, Some(&p)).expect("study");
        assert_eq!((r.model.as_str(), r.pick_dropped), ("demo-claude", true));
        let gone = Pick {
            connection_id: 99,
            model: "x".into(),
        };
        let r = v.resolve(Slot::Chat, Some(&gone)).expect("chat");
        assert!(r.pick_dropped);
        assert_eq!(r.fallback, Fallback::Default);
        // nothing resolves even without the pick
        let mut none = view();
        none.slots = [None; 6];
        assert!(none.resolve(Slot::Chat, Some(&gone)).is_err());
    }

    #[test]
    fn model_id_rules() {
        assert!(model_id_ok("demo-model:latest"));
        assert!(model_id_ok(&"m".repeat(200)));
        assert!(!model_id_ok(&"m".repeat(201)));
        assert!(!model_id_ok(""));
        assert!(!model_id_ok(" demo"));
        assert!(!model_id_ok("demo "));
        assert!(!model_id_ok("de\nmo"));
        assert!(model_id_ok("de mo"), "inner spaces are allowed");
    }

    #[test]
    fn pick_text_and_json_with_colons() {
        let p = Pick::parse_text("2:vendor:model:v1").expect("pick");
        assert_eq!((p.connection_id, p.model.as_str()), (2, "vendor:model:v1"));
        assert_eq!(p.to_text(), "2:vendor:model:v1");
        assert_eq!(
            p.to_json(),
            json!({"connectionId": 2, "model": "vendor:model:v1"})
        );
        for bad in [
            "",
            "x",
            "2",
            "2:",
            ":m",
            "0:m",
            "-1:m",
            "a:m",
            "2: m",
            "99999999999999999999:m",
        ] {
            assert_eq!(Pick::parse_text(bad), None, "{bad:?}");
        }
        assert_eq!(Pick::from_json(&Value::Null).ok(), Some(None));
        let ok = Pick::from_json(&json!({"connectionId": 4, "model": "a:b"}))
            .expect("json")
            .expect("some");
        assert_eq!(ok.to_text(), "4:a:b");
        for bad in [
            json!("1:m"),
            json!(5),
            json!({}),
            json!({"connectionId": 1}),
            json!({"connectionId": 0, "model": "m"}),
            json!({"connectionId": "1", "model": "m"}),
            json!({"connectionId": 1, "model": ""}),
            json!({"connectionId": 1, "model": " m"}),
            json!({"connectionId": 1, "model": "m", "x": 1}),
        ] {
            let e = Pick::from_json(&bad).expect_err("bad pick");
            assert_eq!((e.status.as_u16(), e.code), (400, "ai.badModel"), "{bad}");
        }
    }
}
