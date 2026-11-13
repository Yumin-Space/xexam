//! The backend's generation routes (`/internal/ai/gen/*`, `/internal/ai/import/check`): strict parsing of their answers
//! (API-I1 .. I5). `{"ok": false, "reason": r}` is a refusal; anything that does not match its schema is `Unreadable`
//! (the caller fails only its own request).
use crate::gen::{Diff, Kind, SeedFull, SourceItem, Stratum, Tri};
use serde_json::Value;

/// A refusal reason of the generation routes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Session,
    NotAllowed,
    NotFound,
    AiSource,
    /// `tooFew` with the number of source questions found
    TooFew(i64),
    /// `create` only: no such user (or stamp), or disabled
    User,
}

/// A good answer is parsed; a refusal is a `Reason`; the rest is unreadable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    Refused(Reason),
    Unreadable,
}

fn refusal(v: &Value) -> Option<Reason> {
    let o = v.as_object()?;
    if o.get("ok") != Some(&Value::Bool(false)) {
        return None;
    }
    match o.get("reason")?.as_str()? {
        "session" => Some(Reason::Session),
        "notAllowed" => Some(Reason::NotAllowed),
        "notFound" => Some(Reason::NotFound),
        "aiSource" => Some(Reason::AiSource),
        "user" => Some(Reason::User),
        "tooFew" => Some(Reason::TooFew(o.get("have")?.as_i64()?)),
        _ => None,
    }
}

fn parse_json(bytes: &[u8]) -> Result<Value, WireError> {
    serde_json::from_slice(bytes).map_err(|_| WireError::Unreadable)
}

fn ok_object(bytes: &[u8]) -> Result<serde_json::Map<String, Value>, WireError> {
    let v = parse_json(bytes)?;
    if let Some(r) = refusal(&v) {
        return Err(WireError::Refused(r));
    }
    match v {
        Value::Object(m) if m.get("ok") == Some(&Value::Bool(true)) => Ok(m),
        _ => Err(WireError::Unreadable),
    }
}

fn tri(v: &Value) -> Tri {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Tri {
        en: s("en"),
        zh: s("zh"),
        ja: s("ja"),
    }
}

fn kind_of(s: &str) -> Option<Kind> {
    match s {
        "single" => Some(Kind::Single),
        "multi" => Some(Kind::Multi),
        _ => None,
    }
}

fn diff_of(v: Option<&Value>) -> Option<Diff> {
    match v.and_then(Value::as_str) {
        Some("easy") => Some(Diff::Easy),
        Some("mid") => Some(Diff::Mid),
        Some("hard") => Some(Diff::Hard),
        _ => None,
    }
}

/// The user a session belongs to (API-I1 / I5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUser {
    pub id: i64,
    pub username: String,
    pub role: String,
}

fn user_of(m: &serde_json::Map<String, Value>) -> Result<SessionUser, WireError> {
    let u = m.get("user").ok_or(WireError::Unreadable)?;
    let id = u.get("id").and_then(Value::as_i64).filter(|n| *n > 0);
    let name = u.get("username").and_then(Value::as_str);
    let role = u.get("role").and_then(Value::as_str);
    match (id, name, role) {
        (Some(id), Some(n), Some(r)) if r == "admin" || r == "user" => Ok(SessionUser {
            id,
            username: n.to_string(),
            role: r.to_string(),
        }),
        _ => Err(WireError::Unreadable),
    }
}

/// `/internal/ai/import/check` (API-I5).
pub fn parse_import_check(bytes: &[u8]) -> Result<SessionUser, WireError> {
    user_of(&ok_object(bytes)?)
}

/// [`parse_import_check`] for a request that carried a bank name: the user and whether that name is already taken
/// (`nameTaken`; an answer without it counts as free).
pub fn parse_import_check_named(bytes: &[u8]) -> Result<(SessionUser, bool), WireError> {
    let m = ok_object(bytes)?;
    let taken = m.get("nameTaken").and_then(Value::as_bool).unwrap_or(false);
    Ok((user_of(&m)?, taken))
}

/// A good `/internal/ai/gen/source` answer (API-I1).
#[derive(Debug, Clone)]
pub struct SourceReply {
    pub user: SessionUser,
    pub total: i64,
    pub banks: Vec<(i64, String)>,
    pub strata: Vec<Stratum>,
    pub items: Vec<SourceItem>,
    /// Stems of earlier AI banks (en, zh) against repeats
    pub previous: Vec<Tri>,
}

pub fn parse_source(bytes: &[u8]) -> Result<SourceReply, WireError> {
    let m = ok_object(bytes)?;
    let user = user_of(&m)?;
    let total = m
        .get("total")
        .and_then(Value::as_i64)
        .ok_or(WireError::Unreadable)?;
    let arr = |k: &str| -> Result<&Vec<Value>, WireError> {
        m.get(k)
            .and_then(Value::as_array)
            .ok_or(WireError::Unreadable)
    };
    let mut banks = Vec::new();
    for b in arr("banks")? {
        let id = b.get("id").and_then(Value::as_i64);
        let name = b.get("name").and_then(Value::as_str);
        match (id, name) {
            (Some(id), Some(n)) => banks.push((id, n.to_string())),
            _ => return Err(WireError::Unreadable),
        }
    }
    let mut strata = Vec::new();
    for s in arr("strata")? {
        let bank = s.get("bank").and_then(Value::as_i64);
        let sheet = s.get("sheet").and_then(Value::as_str);
        let n = s.get("n").and_then(Value::as_u64);
        let multi = s.get("multi").and_then(Value::as_u64);
        match (bank, sheet, n, multi) {
            (Some(bank), Some(sheet), Some(n), Some(multi)) => strata.push(Stratum {
                bank,
                sheet: sheet.to_string(),
                n: n as usize,
                multi: multi as usize,
            }),
            _ => return Err(WireError::Unreadable),
        }
    }
    let mut items = Vec::new();
    for i in arr("items")? {
        let id = i.get("id").and_then(Value::as_str);
        let s = i.get("s").and_then(Value::as_u64);
        let kind = i.get("kind").and_then(Value::as_str).and_then(kind_of);
        let opts = i.get("opts").and_then(Value::as_u64);
        let right = i.get("right").and_then(Value::as_u64);
        match (id, s, kind, opts, right) {
            (Some(id), Some(s), Some(kind), Some(opts), Some(right))
                if (s as usize) < strata.len() && opts <= 255 && right <= 255 =>
            {
                items.push(SourceItem {
                    id: id.to_string(),
                    stratum: s as usize,
                    kind,
                    opts: opts as u8,
                    right: right as u8,
                    diff: diff_of(i.get("diff")),
                    stem: tri(i.get("stem").unwrap_or(&Value::Null)),
                });
            }
            _ => return Err(WireError::Unreadable),
        }
    }
    let previous = m
        .get("previous")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(tri).collect())
        .unwrap_or_default();
    Ok(SourceReply {
        user,
        total,
        banks,
        strata,
        items,
        previous,
    })
}

/// A good `/internal/ai/gen/seeds` answer (API-I2): the seeds by id.
pub fn parse_seeds(bytes: &[u8]) -> Result<Vec<SeedFull>, WireError> {
    let m = ok_object(bytes)?;
    let items = m
        .get("items")
        .and_then(Value::as_array)
        .ok_or(WireError::Unreadable)?;
    let mut out = Vec::new();
    for i in items {
        let id = i.get("id").and_then(Value::as_str);
        let kind = i.get("kind").and_then(Value::as_str).and_then(kind_of);
        let sheet = i.get("sheet").and_then(Value::as_str);
        let opts = i.get("options").and_then(Value::as_array);
        let answer = i.get("answer").and_then(Value::as_array);
        let (Some(id), Some(kind), Some(sheet), Some(opts), Some(answer)) =
            (id, kind, sheet, opts, answer)
        else {
            return Err(WireError::Unreadable);
        };
        let options: Vec<Tri> = opts.iter().map(tri).collect();
        let mut pos: Vec<u8> = Vec::new();
        for a in answer {
            let letter = a.as_str().unwrap_or("");
            let b = letter.as_bytes();
            if b.len() != 1 || !b[0].is_ascii_uppercase() {
                return Err(WireError::Unreadable);
            }
            pos.push(b[0] - b'A');
        }
        out.push(SeedFull {
            id: id.to_string(),
            sheet: sheet.to_string(),
            kind,
            stem: tri(i.get("stem").unwrap_or(&Value::Null)),
            options,
            answer: pos,
            explanation: tri(i.get("explanation").unwrap_or(&Value::Null)),
            diff: diff_of(i.get("diff")),
        });
    }
    Ok(out)
}

/// A good `/internal/ai/gen/create` answer (API-I3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateReply {
    pub bank_id: i64,
    pub name: String,
    pub created: bool,
}

pub fn parse_create(bytes: &[u8]) -> Result<CreateReply, WireError> {
    let m = ok_object(bytes)?;
    match (
        m.get("bankId").and_then(Value::as_i64),
        m.get("name").and_then(Value::as_str),
        m.get("created").and_then(Value::as_bool),
    ) {
        (Some(bank_id), Some(name), Some(created)) => Ok(CreateReply {
            bank_id,
            name: name.to_string(),
            created,
        }),
        _ => Err(WireError::Unreadable),
    }
}

/// A good `/internal/ai/gen/lookup` answer (API-I4): the bank of a job key, if any.
pub fn parse_lookup(bytes: &[u8]) -> Result<Option<CreateReply>, WireError> {
    let m = ok_object(bytes)?;
    match m.get("bank") {
        Some(Value::Null) => Ok(None),
        Some(b) => match (
            b.get("id").and_then(Value::as_i64),
            b.get("name").and_then(Value::as_str),
        ) {
            (Some(bank_id), Some(name)) => Ok(Some(CreateReply {
                bank_id,
                name: name.to_string(),
                created: false,
            })),
            _ => Err(WireError::Unreadable),
        },
        None => Err(WireError::Unreadable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn b(v: Value) -> Vec<u8> {
        v.to_string().into_bytes()
    }

    #[test]
    fn refusals() {
        for (reason, want) in [
            ("session", Reason::Session),
            ("notAllowed", Reason::NotAllowed),
            ("notFound", Reason::NotFound),
            ("aiSource", Reason::AiSource),
            ("user", Reason::User),
        ] {
            assert_eq!(
                parse_import_check(&b(json!({"ok": false, "reason": reason}))),
                Err(WireError::Refused(want))
            );
        }
        assert_eq!(
            parse_source(&b(json!({"ok": false, "reason": "tooFew", "have": 18}))).map(|_| ()),
            Err(WireError::Refused(Reason::TooFew(18)))
        );
        assert_eq!(
            parse_import_check(&b(json!({"ok": false, "reason": "weird"}))),
            Err(WireError::Unreadable)
        );
        assert_eq!(parse_import_check(b"not json"), Err(WireError::Unreadable));
    }

    #[test]
    fn import_check() {
        let u = parse_import_check(&b(
            json!({"ok": true, "user": {"id": 12, "username": "alexandra_k", "role": "user"}}),
        ))
        .expect("ok");
        assert_eq!(
            (u.id, u.username.as_str(), u.role.as_str()),
            (12, "alexandra_k", "user")
        );
        assert_eq!(
            parse_import_check(&b(
                json!({"ok": true, "user": {"id": 0, "username": "x", "role": "user"}})
            )),
            Err(WireError::Unreadable)
        );
    }

    #[test]
    fn a_source_answer() {
        let ans = json!({
            "ok": true,
            "user": {"id": 12, "username": "alexandra_k", "role": "user"},
            "total": 57,
            "banks": [{"id": 3, "name": "Kitchen basics"}],
            "strata": [{"bank": 3, "sheet": "Ovens", "n": 30, "multi": 4}],
            "items": [{"id": "1f2e3d4c5b6a", "s": 0, "kind": "single", "opts": 4, "right": 1, "diff": "mid",
                       "stem": {"en": "Which tool?", "zh": "哪种工具？", "ja": "どの道具？"}},
                      {"id": "aa", "s": 0, "kind": "multi", "opts": 5, "right": 2, "diff": null,
                       "stem": {"en": "x", "zh": "", "ja": ""}}],
            "previous": [{"en": "old", "zh": "旧"}]
        });
        let r = parse_source(&b(ans.clone())).expect("ok");
        assert_eq!(r.total, 57);
        assert_eq!(r.banks, vec![(3, "Kitchen basics".to_string())]);
        assert_eq!(r.strata[0].n, 30);
        assert_eq!(r.items[0].diff, Some(Diff::Mid));
        assert_eq!(r.items[1].diff, None);
        assert_eq!(r.items[1].kind, Kind::Multi);
        assert_eq!(r.previous[0].en, "old");
        assert_eq!(r.previous[0].ja, "");
        // a stratum index out of range, an unknown kind
        let mut bad = ans.clone();
        bad["items"][0]["s"] = json!(7);
        assert_eq!(
            parse_source(&b(bad)).map(|_| ()),
            Err(WireError::Unreadable)
        );
        let mut bad = ans;
        bad["items"][0]["kind"] = json!("hotspot");
        assert_eq!(
            parse_source(&b(bad)).map(|_| ()),
            Err(WireError::Unreadable)
        );
    }

    #[test]
    fn seeds_answer_letters_become_positions() {
        let ans = json!({"ok": true, "items": [{
            "id": "abc", "kind": "single", "sheet": "Ovens", "diff": "hard",
            "stem": {"en": "Q", "zh": "题", "ja": "問"},
            "options": [{"key": "A", "en": "a", "zh": "甲", "ja": "あ"}, {"key": "B", "en": "b", "zh": "乙", "ja": "い"}],
            "answer": ["B"],
            "explanation": {"en": "e", "zh": "解", "ja": "説"}}]});
        let s = parse_seeds(&b(ans.clone())).expect("ok");
        assert_eq!(s[0].answer, vec![1]);
        assert_eq!(s[0].options.len(), 2);
        assert_eq!(s[0].diff, Some(Diff::Hard));
        let mut bad = ans;
        bad["items"][0]["answer"] = json!(["b"]);
        assert_eq!(parse_seeds(&b(bad)), Err(WireError::Unreadable));
    }

    #[test]
    fn create_and_lookup() {
        let c = parse_create(&b(
            json!({"ok": true, "bankId": 41, "name": "N", "created": false}),
        ))
        .expect("ok");
        assert_eq!((c.bank_id, c.created), (41, false));
        assert_eq!(
            parse_create(&b(json!({"ok": false, "reason": "notAllowed"}))),
            Err(WireError::Refused(Reason::NotAllowed))
        );
        assert_eq!(
            parse_lookup(&b(json!({"ok": true, "bank": null}))),
            Ok(None)
        );
        let l = parse_lookup(&b(
            json!({"ok": true, "bank": {"id": 4, "name": "x", "made": 20, "goal": 20}}),
        ))
        .expect("ok")
        .expect("bank");
        assert_eq!(l.bank_id, 4);
        assert_eq!(
            parse_lookup(&b(json!({"ok": true}))),
            Err(WireError::Unreadable)
        );
    }
}
