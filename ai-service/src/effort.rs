//! Thinking strength per feature (v1.6): which request parameter a slot sends, and at which level.
//!
//! The choice only decides the parameter put into the request body; it is never checked against the connection's
//! wire format (a refusal is handled by the provider fallback). `None` everywhere means "send nothing".
use serde_json::{json, Value};

/// The parameter family of a thinking-strength choice (what goes into the request body).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EffortFamily {
    Anthropic,
    OpenAi,
    Google,
    Responses,
    Gemma,
}

/// Levels per family (official docs, 2026-11-16). Order = UI order.
pub const ANTHROPIC_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
pub const OPENAI_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
/// Google `thinking_level`, written uppercase as Google writes it.
pub const GOOGLE_LEVELS: [&str; 5] = ["MINIMAL", "LOW", "MEDIUM", "HIGH", "XHIGH"];
/// Responses-API style `reasoning.effort`: the full set, weakest to strongest.
pub const RESPONSES_LEVELS: [&str; 7] =
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
/// Gemma 4 on Google accepts only these two `thinking_level` values: HIGH = thinking, MINIMAL = no thinking.
pub const GEMMA_LEVELS: [&str; 2] = ["HIGH", "MINIMAL"];

impl EffortFamily {
    pub const ALL: [EffortFamily; 5] = [
        Self::Anthropic,
        Self::OpenAi,
        Self::Google,
        Self::Responses,
        Self::Gemma,
    ];

    /// `anthropic | openai | google | responses | gemma`
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Google => "google",
            Self::Responses => "responses",
            Self::Gemma => "gemma",
        }
    }

    /// Exact, lowercase only.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.as_str() == s)
    }

    pub fn levels(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic => &ANTHROPIC_LEVELS,
            Self::OpenAi => &OPENAI_LEVELS,
            Self::Google => &GOOGLE_LEVELS,
            Self::Responses => &RESPONSES_LEVELS,
            Self::Gemma => &GEMMA_LEVELS,
        }
    }

    /// Name of the body field, for the log line.
    pub fn param(self) -> &'static str {
        match self {
            Self::Anthropic => "output_config.effort",
            Self::OpenAi => "reasoning_effort",
            Self::Responses => "reasoning.effort",
            Self::Google | Self::Gemma => "extra_body.google.thinking_config.thinking_level",
        }
    }
}

/// A validated choice. The fields are private, so a value can only come from `new` / `parse_tag`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Effort {
    family: EffortFamily,
    level: &'static str,
}

impl Effort {
    /// `Some` only when `level` is exactly one of `family.levels()`; the stored str is the const entry.
    pub fn new(family: EffortFamily, level: &str) -> Option<Effort> {
        let level = family.levels().iter().copied().find(|l| *l == level)?;
        Some(Effort { family, level })
    }

    pub fn family(self) -> EffortFamily {
        self.family
    }

    pub fn level(self) -> &'static str {
        self.level
    }

    /// `anthropic:high`: the DB value and the memo-key suffix.
    pub fn tag(self) -> String {
        format!("{}:{}", self.family.as_str(), self.level)
    }

    /// Splits once on `:`; anything invalid is `None`. Legacy stored values stay readable: Google levels
    /// case-insensitively (`google:low` = `LOW`; `google:none` and the removed OpenAI `none` / `minimal` are `None`),
    /// `gemma:true` / `gemma:false` = HIGH / MINIMAL.
    pub fn parse_tag(s: &str) -> Option<Effort> {
        let (f, l) = s.split_once(':')?;
        // legacy: the family was called `deepseek` before it became `responses`
        let family = EffortFamily::parse(if f == "deepseek" { "responses" } else { f })?;
        match family {
            EffortFamily::Google => Effort::new(family, &l.to_ascii_uppercase()),
            EffortFamily::Gemma => match l {
                "true" => Effort::new(family, "HIGH"),
                "false" => Effort::new(family, "MINIMAL"),
                _ => Effort::new(family, l),
            },
            _ => Effort::new(family, l),
        }
    }

    /// `{"family": "anthropic", "level": "high"}`
    pub fn to_json(self) -> Value {
        json!({"family": self.family.as_str(), "level": self.level})
    }
}

/// `{"anthropic": [...], "openai": [...], "google": [...], "responses": [...], "gemma": [...]}`: the level lists for the settings answer.
pub fn levels_json() -> Value {
    json!({
        "anthropic": ANTHROPIC_LEVELS,
        "openai": OPENAI_LEVELS,
        "google": GOOGLE_LEVELS,
        "responses": RESPONSES_LEVELS,
        "gemma": GEMMA_LEVELS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_new_accepts_every_listed_level() {
        for f in EffortFamily::ALL {
            for l in f.levels() {
                let e = Effort::new(f, l).expect("listed level");
                assert_eq!(e.level(), *l);
                assert_eq!(e.family(), f);
            }
        }
    }

    #[test]
    fn effort_new_rejects_level_outside_family() {
        use EffortFamily::*;
        for (f, l) in [
            (Anthropic, "none"),
            (Anthropic, "minimal"),
            (OpenAi, "none"),
            (OpenAi, "minimal"),
            (Google, "max"),
            (Google, "none"),
            (Google, "low"),
            (Responses, "turbo"),
            (Responses, "HIGH"),
            (Gemma, "high"),
            (Gemma, "true"),
            (Gemma, "none"),
            (Google, "LOW "),
            (OpenAi, "High"),
            (OpenAi, " high"),
            (OpenAi, ""),
        ] {
            assert!(Effort::new(f, l).is_none(), "{f:?} {l:?}");
        }
    }

    #[test]
    fn effort_tag_round_trip() {
        for f in EffortFamily::ALL {
            for l in f.levels() {
                let e = Effort::new(f, l).unwrap();
                assert_eq!(Effort::parse_tag(&e.tag()), Some(e));
            }
        }
        assert_eq!(
            Effort::new(EffortFamily::Anthropic, "high").unwrap().tag(),
            "anthropic:high"
        );
    }

    #[test]
    fn parse_tag_rejects_garbage() {
        for s in ["x", "openai:", "default", "anthropic:high:x", "", ":high"] {
            assert!(Effort::parse_tag(s).is_none(), "{s:?}");
        }
    }

    #[test]
    fn level_lists_match_research() {
        assert_eq!(ANTHROPIC_LEVELS, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(OPENAI_LEVELS, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(GOOGLE_LEVELS, ["MINIMAL", "LOW", "MEDIUM", "HIGH", "XHIGH"]);
        assert_eq!(
            RESPONSES_LEVELS,
            ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(GEMMA_LEVELS, ["HIGH", "MINIMAL"]);
        // Responses family: seven levels; others read as None; legacy deepseek:* maps over those it matches
        assert_eq!(
            Effort::parse_tag("responses:none").unwrap().tag(),
            "responses:none"
        );
        assert_eq!(
            Effort::parse_tag("responses:xhigh").unwrap().tag(),
            "responses:xhigh"
        );
        assert!(Effort::parse_tag("responses:turbo").is_none());
        assert_eq!(
            Effort::parse_tag("deepseek:low").unwrap().tag(),
            "responses:low"
        );
        assert_eq!(
            Effort::parse_tag("deepseek:medium").unwrap().tag(),
            "responses:medium"
        );
        assert!(Effort::parse_tag("deepseek:turbo").is_none());
        let j = levels_json();
        assert_eq!(j["gemma"], json!(["HIGH", "MINIMAL"]));
        assert_eq!(j["responses"].as_array().unwrap().len(), 7);
        assert_eq!(j["openai"].as_array().unwrap().len(), 5);
        assert_eq!(j["google"].as_array().unwrap().len(), 5);
        assert_eq!(
            Effort::new(EffortFamily::Google, "HIGH").unwrap().to_json(),
            json!({"family": "google", "level": "HIGH"})
        );
    }

    #[test]
    fn legacy_tags_still_parse() {
        let t = |s: &str| Effort::parse_tag(s).map(|e| e.tag());
        assert_eq!(t("google:low").as_deref(), Some("google:LOW"));
        assert_eq!(t("google:xHigh").as_deref(), Some("google:XHIGH"));
        assert_eq!(t("google:MEDIUM").as_deref(), Some("google:MEDIUM"));
        assert_eq!(t("google:minimal").as_deref(), Some("google:MINIMAL"));
        assert_eq!(t("google:none"), None);
        assert_eq!(t("gemma:true").as_deref(), Some("gemma:HIGH"));
        assert_eq!(t("gemma:false").as_deref(), Some("gemma:MINIMAL"));
        assert_eq!(t("gemma:HIGH").as_deref(), Some("gemma:HIGH"));
        assert_eq!(t("openai:none"), None);
        assert_eq!(t("openai:minimal"), None);
        assert_eq!(t("openai:xhigh").as_deref(), Some("openai:xhigh"));
    }
}
