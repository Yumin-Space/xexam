//! Per-user AI+ feature switches (spec FP-1, FP-2, addendum P-9 / P-10): which AI features an administrator allowed for a
//! plain user. The AI database keeps one integer per user (`users.features_off`): a set bit means the feature is OFF, so
//! 0 (and a user without a row) means everything is allowed. Administrators always have every feature.
use serde_json::{Map, Value};

/// An AI+ feature that can be switched off for one user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// AI Chat: send, retry, the memory suggestion
    Chat,
    /// Practice hints
    Hint,
    /// Practice explanations and result / record analysis
    Explain,
    /// AI summary of a record
    Summary,
    /// AI bank generation (Bank management action and both Home buttons)
    Generate,
    /// AI file import (upload and confirm)
    Import,
}

/// Every feature, in the order of the JSON object.
pub const ALL: [Feature; 6] = [
    Feature::Chat,
    Feature::Hint,
    Feature::Explain,
    Feature::Summary,
    Feature::Generate,
    Feature::Import,
];

/// The bits that exist; a stored value may only use these.
pub const KNOWN_BITS: i64 = 63;

impl Feature {
    pub fn bit(self) -> i64 {
        match self {
            Feature::Chat => 1,
            Feature::Hint => 2,
            Feature::Explain => 4,
            Feature::Summary => 8,
            Feature::Generate => 16,
            Feature::Import => 32,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Feature::Chat => "chat",
            Feature::Hint => "hint",
            Feature::Explain => "explain",
            Feature::Summary => "summary",
            Feature::Generate => "generate",
            Feature::Import => "import",
        }
    }

    pub fn parse(s: &str) -> Option<Feature> {
        ALL.iter().copied().find(|f| f.key() == s)
    }
}

/// `true` when `f` is allowed for a user whose `features_off` is `off`.
pub fn allowed(off: i64, f: Feature) -> bool {
    off & f.bit() == 0
}

/// `{"chat": true, …}` with the six keys in order; `off` = the stored bits.
pub fn to_json(off: i64) -> Value {
    let mut m = Map::new();
    for f in ALL {
        m.insert(f.key().to_string(), Value::Bool(allowed(off, f)));
    }
    Value::Object(m)
}

/// Every feature on (what administrators and users without a row have).
pub fn all_on_json() -> Value {
    to_json(0)
}

/// An object that is not exactly the six boolean keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadFeatures;

/// The stored bits for a JSON object with exactly the six keys, all booleans; anything else is refused.
pub fn from_json(m: &Map<String, Value>) -> Result<i64, BadFeatures> {
    if m.len() != ALL.len() {
        return Err(BadFeatures);
    }
    let mut off = 0;
    for f in ALL {
        match m.get(f.key()) {
            Some(Value::Bool(true)) => {}
            Some(Value::Bool(false)) => off |= f.bit(),
            _ => return Err(BadFeatures),
        }
    }
    Ok(off)
}

/// `"hint off, generate on"` for the features whose state differs between two stored values (audit line), or `None`.
pub fn describe_change(before: i64, after: i64) -> Option<String> {
    let parts: Vec<String> = ALL
        .iter()
        .filter(|f| allowed(before, **f) != allowed(after, **f))
        .map(|f| {
            format!(
                "{} {}",
                f.key(),
                if allowed(after, *f) { "on" } else { "off" }
            )
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn six_bits_and_keys() {
        let bits: Vec<i64> = ALL.iter().map(|f| f.bit()).collect();
        assert_eq!(bits, vec![1, 2, 4, 8, 16, 32]);
        let keys: Vec<&str> = ALL.iter().map(|f| f.key()).collect();
        assert_eq!(
            keys,
            vec!["chat", "hint", "explain", "summary", "generate", "import"]
        );
        for f in ALL {
            assert_eq!(Feature::parse(f.key()), Some(f));
        }
        assert_eq!(Feature::parse("Chat"), None);
        assert_eq!(Feature::parse("other"), None);
        assert_eq!(KNOWN_BITS, bits.iter().sum::<i64>());
    }

    #[test]
    fn allowed_follows_the_bits() {
        assert!(ALL.iter().all(|f| allowed(0, *f)));
        assert!(!allowed(2, Feature::Hint));
        assert!(allowed(2, Feature::Chat));
        assert!(!allowed(32, Feature::Import));
        assert!(allowed(32, Feature::Generate));
        assert!(!allowed(16, Feature::Generate));
        assert!(allowed(16, Feature::Import));
    }

    #[test]
    fn json_round_trip_and_refusals() {
        assert_eq!(
            serde_json::to_string(&to_json(0)).expect("json"),
            r#"{"chat":true,"hint":true,"explain":true,"summary":true,"generate":true,"import":true}"#
        );
        for off in 0..=KNOWN_BITS {
            let v = to_json(off);
            let m = v.as_object().cloned().unwrap_or_default();
            assert_eq!(from_json(&m), Ok(off));
        }
        let ok = |v: Value| from_json(&v.as_object().cloned().unwrap_or_default());
        // five keys (the v1.0 shape), a seventh, a wrong type, an unknown key
        assert!(ok(
            json!({"chat": true, "hint": true, "explain": true, "summary": true, "generate": true})
        )
        .is_err());
        assert!(ok(json!({"chat": true, "hint": true, "explain": true, "summary": true, "generate": true, "import": true, "x": true})).is_err());
        assert!(ok(json!({"chat": 1, "hint": true, "explain": true, "summary": true, "generate": true, "import": true})).is_err());
        assert!(ok(json!({"chat": true, "hint": true, "explain": true, "summary": true, "generate": true, "other": true})).is_err());
    }

    #[test]
    fn the_audit_description_names_only_the_changes() {
        assert_eq!(describe_change(0, 0), None);
        assert_eq!(
            describe_change(0, 2 | 32).as_deref(),
            Some("hint off, import off")
        );
        assert_eq!(
            describe_change(2 | 16, 16 | 4).as_deref(),
            Some("hint on, explain off")
        );
    }
}
