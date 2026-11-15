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
    DeepSeek,
    Gemma,
}

/// Levels per family (official docs, 2026-11-16). Order = UI order.
pub const ANTHROPIC_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
pub const OPENAI_LEVELS: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
pub const GOOGLE_LEVELS: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];
/// DeepSeek thinking mode (official docs): none / low / high / max. Thinking is on by default, effort high.
pub const DEEPSEEK_LEVELS: [&str; 4] = ["none", "low", "high", "max"];
/// Gemma: `chat_template_kwargs.enable_thinking`, a boolean; the levels are its two values.
pub const GEMMA_LEVELS: [&str; 2] = ["true", "false"];

impl EffortFamily {
    pub const ALL: [EffortFamily; 5] = [
        Self::Anthropic,
        Self::OpenAi,
        Self::Google,
        Self::DeepSeek,
        Self::Gemma,
    ];

    /// `anthropic | openai | google | deepseek | gemma`
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Google => "google",
            Self::DeepSeek => "deepseek",
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
            Self::DeepSeek => &DEEPSEEK_LEVELS,
            Self::Gemma => &GEMMA_LEVELS,
        }
    }

    /// Name of the body field, for the log line.
    pub fn param(self) -> &'static str {
        match self {
            Self::Anthropic => "output_config.effort",
            Self::OpenAi | Self::Google | Self::DeepSeek => "reasoning_effort",
            Self::Gemma => "chat_template_kwargs.enable_thinking",
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

    /// Splits once on `:`; anything invalid is `None`.
    pub fn parse_tag(s: &str) -> Option<Effort> {
        let (f, l) = s.split_once(':')?;
        Effort::new(EffortFamily::parse(f)?, l)
    }

    /// `{"family": "anthropic", "level": "high"}`
    pub fn to_json(self) -> Value {
        json!({"family": self.family.as_str(), "level": self.level})
    }
}

/// `{"anthropic": [...], "openai": [...], "google": [...], "deepseek": [...], "gemma": [...]}`: the level lists for the settings answer.
pub fn levels_json() -> Value {
    json!({
        "anthropic": ANTHROPIC_LEVELS,
        "openai": OPENAI_LEVELS,
        "google": GOOGLE_LEVELS,
        "deepseek": DEEPSEEK_LEVELS,
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
            (Google, "max"),
            (DeepSeek, "xhigh"),
            (DeepSeek, "medium"),
            (Gemma, "high"),
            (Gemma, "True"),
            (Gemma, "none"),
            (Google, "low "),
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
        assert_eq!(
            OPENAI_LEVELS,
            ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            GOOGLE_LEVELS,
            ["none", "minimal", "low", "medium", "high", "xhigh"]
        );
        assert_eq!(DEEPSEEK_LEVELS, ["none", "low", "high", "max"]);
        assert_eq!(GEMMA_LEVELS, ["true", "false"]);
        assert_eq!(
            Effort::parse_tag("deepseek:none").unwrap().tag(),
            "deepseek:none"
        );
        // the removed DeepSeek level reads as the default (None)
        assert!(Effort::parse_tag("deepseek:xhigh").is_none());
        assert_eq!(
            Effort::parse_tag("gemma:false").unwrap().tag(),
            "gemma:false"
        );
        assert_eq!(levels_json()["gemma"].as_array().unwrap().len(), 2);
        let j = levels_json();
        assert_eq!(j["deepseek"].as_array().unwrap().len(), 4);
        assert_eq!(j["openai"].as_array().unwrap().len(), 7);
        assert_eq!(
            Effort::new(EffortFamily::Google, "high").unwrap().to_json(),
            json!({"family": "google", "level": "high"})
        );
    }
}
