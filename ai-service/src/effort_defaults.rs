//! The level a model uses when nothing is sent (v1.6, official docs) and the level picked for a family (v2.1).
//!
//! An exact port of `frontend/src/domain/effort-defaults.ts`; both read the same fixture table in their tests
//! (`frontend/src/domain/effort-defaults.fixture.json`).

/// Known defaults of a normalized model id (exact lookup, no prefix matching).
const TABLE: &[(&str, &str)] = &[
    ("claude-opus-5-5", "medium"),
    ("claude-haiku-5-5", "medium"),
    ("claude-sonnet-5-5", "high"),
    ("claude-opus-5", "high"),
    ("claude-sonnet-5", "high"),
    ("claude-opus-4-8", "high"),
    ("claude-opus-4-7", "high"),
    ("claude-opus-4-6", "high"),
    ("claude-sonnet-4-6", "high"),
    ("gpt-6.1-sol", "medium"),
    ("gpt-6.1-terra", "medium"),
    ("gpt-6-sol", "medium"),
    ("gpt-6-luna", "medium"),
    ("gpt-5.6", "medium"),
    ("gpt-5.5", "medium"),
    ("gpt-5.4", "none"),
    ("gpt-5.2", "none"),
    ("gpt-5.1", "none"),
    ("gemini-4-hydrogen", "HIGH"),
    ("gemini-3.1-pro-preview", "HIGH"),
    ("gemini-3-flash-preview", "HIGH"),
    ("gemini-3-pro-preview", "HIGH"),
    ("gemini-4-carbon", "MEDIUM"),
    ("gemini-4-argon", "MEDIUM"),
    ("gemini-3.8-flash", "MEDIUM"),
    ("gemini-3.6-flash", "MEDIUM"),
    ("gemini-3.5-flash-lite", "MINIMAL"),
    ("gemini-3.1-flash-lite", "MINIMAL"),
    // Gemini 2.5 Flash-Lite does not think unless asked
    ("gemini-2.5-flash-lite", "none"),
];

/// DeepSeek model ids (Responses API family): effort high unless set.
const DEEPSEEK_DEFAULT: &str = "high";

/// Gemma models (sent through the Google family) think by default: `thinking_level` HIGH.
const GEMMA_DEFAULT: &str = "HIGH";

const ROUTING: [&str; 7] = [
    "us.",
    "eu.",
    "apac.",
    "global.",
    "anthropic.",
    "google.",
    "openai.",
];

/// `-` followed by exactly `n` ASCII digits at the very end: the stripped str, else the input.
fn strip_digits_suffix(m: &str, n: usize) -> &str {
    let b = m.as_bytes();
    if b.len() > n && b[b.len() - n - 1] == b'-' && b[b.len() - n..].iter().all(u8::is_ascii_digit)
    {
        &m[..m.len() - n - 1]
    } else {
        m
    }
}

/// `-vN` (one or more digits) at the very end.
fn strip_version(m: &str) -> &str {
    let digits = m.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return m;
    }
    let rest = &m[..m.len() - digits];
    rest.strip_suffix("-v").unwrap_or(m)
}

/// `-YYYY-MM-DD` at the very end.
fn strip_dashed_date(m: &str) -> &str {
    let b = m.as_bytes();
    let n = b.len();
    if n >= 11
        && b[n - 11] == b'-'
        && b[n - 10..n - 6].iter().all(u8::is_ascii_digit)
        && b[n - 6] == b'-'
        && b[n - 5..n - 3].iter().all(u8::is_ascii_digit)
        && b[n - 3] == b'-'
        && b[n - 2..].iter().all(u8::is_ascii_digit)
    {
        &m[..n - 11]
    } else {
        m
    }
}

/// The model id without routing prefixes, version suffixes and date stamps.
pub fn normalize(model: &str) -> String {
    let lower = model.to_lowercase();
    let mut m = lower.rsplit('/').next().unwrap_or("");
    'strip: loop {
        for p in ROUTING {
            if let Some(rest) = m.strip_prefix(p) {
                m = rest;
                continue 'strip;
            }
        }
        break;
    }
    m = m.split(['@', ':']).next().unwrap_or("");
    m = strip_version(m);
    m = strip_dashed_date(m);
    m = strip_digits_suffix(m, 8);
    if m.starts_with("claude-") {
        m.replace('.', "-")
    } else {
        m.to_string()
    }
}

/// The level a model uses when nothing is sent, or `None` when it is not known (exact lookup, no prefix matching).
pub fn known_default(model: &str) -> Option<&'static str> {
    if model.is_empty() {
        return None;
    }
    let k = normalize(model);
    if k.contains("deepseek") {
        return Some(DEEPSEEK_DEFAULT);
    }
    if k.contains("gemma") {
        return Some(GEMMA_DEFAULT);
    }
    TABLE.iter().find(|(id, _)| *id == k).map(|(_, l)| *l)
}

/// The level for a family: the current one when the family has it (case-insensitive), else the model's known default
/// when the family has it, else the family's middle level (`medium`), or `high` when that is missing, or the first
/// level.
pub fn level_for_family(
    current: Option<&str>,
    levels: &'static [&'static str],
    model_default: Option<&str>,
) -> &'static str {
    let find = |x: &str| levels.iter().copied().find(|l| l.eq_ignore_ascii_case(x));
    current
        .and_then(find)
        .or_else(|| model_default.and_then(find))
        .or_else(|| find("medium"))
        .or_else(|| find("high"))
        .or_else(|| levels.first().copied())
        .unwrap_or("high")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effort::EffortFamily;
    use serde_json::Value;

    fn fixture() -> Option<Value> {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../frontend/src/domain/effort-defaults.fixture.json"
        );
        let text = std::fs::read_to_string(p).ok()?;
        Some(serde_json::from_str(&text).expect("fixture json"))
    }

    #[test]
    fn shared_fixture_known_defaults() {
        let Some(f) = fixture() else { return };
        let rows = f["knownDefault"].as_array().expect("rows");
        assert!(rows.len() > 40);
        for r in rows {
            let m = r[0].as_str().unwrap();
            assert_eq!(known_default(m), r[1].as_str(), "{m:?}");
        }
    }

    #[test]
    fn shared_fixture_levels_and_level_for_family() {
        let Some(f) = fixture() else { return };
        for fam in EffortFamily::ALL {
            let want: Vec<&str> = f["levels"][fam.as_str()]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            assert_eq!(fam.levels(), want.as_slice(), "{fam:?}");
        }
        for r in f["levelForFamily"].as_array().unwrap() {
            let fam = EffortFamily::parse(r["family"].as_str().unwrap()).unwrap();
            let model = r["model"].as_str().unwrap();
            let got = level_for_family(r["current"].as_str(), fam.levels(), known_default(model));
            assert_eq!(got, r["expect"].as_str().unwrap(), "{r}");
        }
    }

    #[test]
    fn normalize_steps() {
        assert_eq!(
            normalize("us.anthropic.claude-opus-5-5-v1:0"),
            "claude-opus-5-5"
        );
        assert_eq!(
            normalize("anthropic/claude-sonnet-4.6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(normalize("claude-opus-5-20260101"), "claude-opus-5");
        assert_eq!(normalize("gpt-5.5-2026-01-02"), "gpt-5.5");
        assert_eq!(normalize("GPT-5.5"), "gpt-5.5");
        assert_eq!(
            normalize("models/gemini-3-flash-preview"),
            "gemini-3-flash-preview"
        );
        assert_eq!(normalize("gemini-3.1"), "gemini-3.1");
    }
}
