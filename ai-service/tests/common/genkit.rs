//! Test kit of AI bank generation and import: the fake backend's generation routes, scripted provider answers (a
//! well-behaved model for every prompt of the service), and made-up files. Everything is invented (a fictional
//! kitchen-equipment course); no real question text.
#![allow(dead_code)]

use super::BackendState;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// What the scripted provider does with one request.
pub enum Reply {
    /// A normal answer
    Text(String),
    /// An answer cut by the token limit
    Cut(String),
    /// A provider error
    Status(u16, String),
    /// The stub's own behaviour
    Fall,
}

/// The scripted provider: (system text, last user turn) → what to answer.
pub type Script = Arc<dyn Fn(&str, &str) -> Reply + Send + Sync>;

/// The generation side of the fake backend.
#[derive(Default)]
pub struct GenBackend {
    pub source_refusal: Option<String>,
    pub too_few_have: i64,
    pub banks: Vec<(i64, String)>,
    pub strata: Vec<Value>,
    pub items: Vec<Value>,
    pub previous: Vec<Value>,
    pub seeds: HashMap<String, Value>,
    /// Bodies of the accepted create calls
    pub created: Vec<Value>,
    pub create_refusal: Option<String>,
    /// Answer create with HTTP 500
    pub create_down: bool,
    pub banks_by_job: HashMap<String, (i64, String)>,
    pub next_bank: i64,
    pub check_refusal: Option<String>,
    /// Bank names the import check reports as taken (`nameTaken`)
    pub taken_names: Vec<String>,
    /// Bodies of the import checks
    pub check_requests: Vec<Value>,
    /// Bodies of the source calls (to see what the service asked)
    pub source_requests: Vec<Value>,
    /// Ids the seeds route leaves out (deleted meanwhile)
    pub seeds_missing: Vec<String>,
}

// —— made-up words ———————————————————————————————————————————————————————————————————————————————————————————————————

const CONS: &[u8] = b"bcdfghjklmprstvwz";
const VOW: &[u8] = b"aeiou";

/// A unique pseudo word (six letters, no real English word).
pub fn pw(n: usize) -> String {
    let mut x = n;
    let mut s = String::new();
    for _ in 0..3 {
        s.push(CONS[x % CONS.len()] as char);
        x /= CONS.len();
        s.push(VOW[x % VOW.len()] as char);
        x /= VOW.len();
    }
    s
}

pub fn en_text(n: usize, words: usize) -> String {
    (0..words)
        .map(|k| pw(n * words + k))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn han(n: usize, len: usize) -> String {
    (0..len)
        .filter_map(|k| char::from_u32(0x4E00 + ((n * len + k) % 20000) as u32))
        .collect()
}

pub fn kana(n: usize, len: usize) -> String {
    (0..len)
        .filter_map(|k| char::from_u32(0x3042 + ((n * len + k) % 80) as u32))
        .collect()
}

fn tri(n: usize, words: usize) -> Value {
    json!({"en": en_text(n, words), "zh": han(n, words), "ja": kana(n, words)})
}

// —— the backend fixture —————————————————————————————————————————————————————————————————————————————————————————————

/// A kitchen course with `sheets` sheets of `per_sheet` single-choice questions each (every fourth is multiple choice).
pub fn kitchen(sheets: usize, per_sheet: usize) -> GenBackend {
    let mut g = GenBackend {
        banks: vec![(3, "Kitchen basics".to_string())],
        next_bank: 41,
        ..GenBackend::default()
    };
    let names = ["Ovens", "Knives", "Storage", "Pans"];
    let mut n = 0usize;
    for s in 0..sheets {
        let sheet = names[s % names.len()];
        let multi_count = (0..per_sheet).filter(|k| k % 4 == 3).count();
        g.strata
            .push(json!({"bank": 3, "sheet": sheet, "n": per_sheet, "multi": multi_count}));
        for k in 0..per_sheet {
            n += 1;
            let multi = k % 4 == 3;
            let id = format!("s{n:03}");
            let opts = if multi { 5 } else { 4 };
            let right = if multi { 2 } else { 1 };
            g.items.push(json!({
                "id": id, "s": s, "kind": if multi { "multi" } else { "single" },
                "opts": opts, "right": right, "diff": if k % 3 == 0 { json!("mid") } else { Value::Null },
                "stem": tri(9000 + n, 9),
            }));
            let options: Vec<Value> = (0..opts)
                .map(|o| {
                    let t = tri(8000 + n * 10 + o, 2);
                    json!({"key": ((b'A' + o as u8) as char).to_string(), "en": t["en"], "zh": t["zh"], "ja": t["ja"]})
                })
                .collect();
            let answer: Vec<String> = (0..right)
                .map(|o| ((b'A' + o as u8) as char).to_string())
                .collect();
            g.seeds.insert(
                id.clone(),
                json!({
                    "id": id, "kind": if multi { "multi" } else { "single" }, "sheet": sheet,
                    "diff": if k % 3 == 0 { json!("mid") } else { Value::Null },
                    "stem": tri(9000 + n, 9), "options": options, "answer": answer,
                    "explanation": tri(7000 + n, 8),
                }),
            );
        }
    }
    g
}

fn refusal(reason: &str) -> Value {
    json!({"ok": false, "reason": reason})
}

fn session_user(s: &BackendState, body: &Value) -> Option<Value> {
    let sid = body.get("sid").and_then(Value::as_str).unwrap_or("");
    let u = s
        .sessions
        .get(sid)
        .and_then(|uid| s.users.get(uid))
        .filter(|u| !u.disabled)?;
    Some(json!({"id": u.id, "username": u.username, "role": u.role}))
}

/// The fake backend's answers for `/internal/ai/gen/*` and `/internal/ai/import/check`; `Err(status)` = an HTTP error.
pub fn answer(s: &mut BackendState, path: &str, body: &Value) -> Result<Value, u16> {
    match path {
        "/internal/ai/import/check" => {
            let Some(user) = session_user(s, body) else {
                return Ok(refusal("session"));
            };
            if let Some(r) = &s.gen.check_refusal {
                return Ok(refusal(r));
            }
            s.gen.check_requests.push(body.clone());
            let mut out = json!({"ok": true, "user": user});
            if let Some(n) = body.get("name").and_then(Value::as_str) {
                out["nameTaken"] = json!(s.gen.taken_names.iter().any(|t| t == n.trim()));
            }
            Ok(out)
        }
        "/internal/ai/gen/source" => {
            s.gen.source_requests.push(body.clone());
            let Some(user) = session_user(s, body) else {
                return Ok(refusal("session"));
            };
            if let Some(r) = &s.gen.source_refusal {
                return Ok(refusal(r));
            }
            let min = body.get("min").and_then(Value::as_i64).unwrap_or(0);
            let total = s.gen.items.len() as i64;
            if total < min {
                return Ok(json!({"ok": false, "reason": "tooFew", "have": total}));
            }
            Ok(json!({
                "ok": true, "user": user, "total": total,
                "banks": s.gen.banks.iter().map(|(id, n)| json!({"id": id, "name": n})).collect::<Vec<_>>(),
                "strata": s.gen.strata, "items": s.gen.items, "previous": s.gen.previous,
            }))
        }
        "/internal/ai/gen/seeds" => {
            if session_user(s, body).is_none() {
                return Ok(refusal("session"));
            }
            let ids: Vec<String> = body
                .get("ids")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let items: Vec<Value> = ids
                .iter()
                .filter(|id| !s.gen.seeds_missing.contains(id))
                .filter_map(|id| s.gen.seeds.get(id).cloned())
                .collect();
            Ok(json!({"ok": true, "items": items}))
        }
        "/internal/ai/gen/create" => {
            if s.gen.create_down {
                return Err(500);
            }
            if let Some(r) = &s.gen.create_refusal {
                return Ok(refusal(r));
            }
            let job = body
                .get("job")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let name = body
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some((id, n)) = s.gen.banks_by_job.get(&job) {
                return Ok(json!({"ok": true, "bankId": id, "name": n, "created": false}));
            }
            let id = s.gen.next_bank;
            s.gen.next_bank += 1;
            s.gen.banks_by_job.insert(job, (id, name.clone()));
            s.gen.created.push(body.clone());
            Ok(json!({"ok": true, "bankId": id, "name": name, "created": true}))
        }
        "/internal/ai/gen/lookup" => {
            let job = body.get("job").and_then(Value::as_str).unwrap_or("");
            Ok(match s.gen.banks_by_job.get(job) {
                Some((id, n)) => {
                    json!({"ok": true, "bank": {"id": id, "name": n, "made": 20, "goal": 20}})
                }
                None => json!({"ok": true, "bank": null}),
            })
        }
        _ => Err(404),
    }
}

// —— a well-behaved model ————————————————————————————————————————————————————————————————————————————————————————————

static COUNTER: AtomicUsize = AtomicUsize::new(1);

fn slots_of(user: &str) -> Vec<Value> {
    // the conversion prompt starts with the list, the other prompts have it after the sources
    let start = if user.starts_with("SLOTS\n") {
        6
    } else {
        match user.find("\nSLOTS\n") {
            Some(at) => at + 7,
            None => return Vec::new(),
        }
    };
    let rest = &user[start..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    serde_json::from_str::<Vec<Value>>(&rest[..end]).unwrap_or_default()
}

fn question_json(slot: &Value, options: usize, correct: usize) -> Value {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let opts: Vec<Value> = (0..options)
        .map(|k| {
            let t = tri(3000 + n * 7 + k, 2);
            json!({"en": t["en"], "zh": t["zh"], "ja": t["ja"], "correct": k < correct})
        })
        .collect();
    json!({
        "slot": slot["slot"],
        "stem": tri(1000 + n, 10),
        "options": opts,
        "explanation": tri(2000 + n, 8),
    })
}

/// Valid questions for the slots of a generation / material prompt.
pub fn generation_answer(user: &str) -> String {
    let qs: Vec<Value> = slots_of(user)
        .iter()
        .map(|s| {
            let options = s["options"].as_u64().unwrap_or(4) as usize;
            let correct = s["correct"].as_u64().unwrap_or(1) as usize;
            question_json(s, options, correct)
        })
        .collect();
    json!({"questions": qs}).to_string()
}

/// The regions of a conversion prompt: `(Q label, text)` in order.
fn regions_of(user: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    for line in user.lines() {
        if let Some(rest) = line.strip_prefix("<<<END Q") {
            if let Some((label, lines)) = cur.take() {
                let _ = rest;
                out.push((label, lines.join("\n")));
            }
        } else if let Some(rest) = line.strip_prefix("<<<Q") {
            let label = format!("Q{}", rest.split(' ').next().unwrap_or(""));
            cur = Some((label, Vec::new()));
        } else if let Some((_, lines)) = cur.as_mut() {
            lines.push(line.to_string());
        }
    }
    out
}

fn strip_number(first: &str) -> String {
    let t = first.trim();
    let t = t.strip_prefix('Q').unwrap_or(t);
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    let t = &t[digits.len()..];
    t.trim_start_matches(['.', ')', ' ']).to_string()
}

/// Faithful conversions for the slots of a conversion prompt (English sources).
pub fn convert_answer(user: &str) -> String {
    let regions = regions_of(user);
    convert_with(user, &|slot| {
        let label = slot["source"].as_str().unwrap_or("");
        regions
            .iter()
            .find(|(l, _)| l == label)
            .map(|(_, t)| t.clone())
            .unwrap_or_default()
    })
}

/// The numbered lines of a whole-file prefix (`L<n>: text`, up to the end fence).
pub fn prefix_lines(user: &str) -> Vec<(usize, String)> {
    let head = user.split("<<<END FILE").next().unwrap_or(user);
    head.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix('L')?;
            let (n, text) = rest.split_once(": ")?;
            Some((n.parse::<usize>().ok()?, text.to_string()))
        })
        .collect()
}

/// The text of `L12-L18` out of the prefix lines.
fn lines_range(all: &[(usize, String)], range: &str) -> String {
    let Some((a, b)) = range.trim_start_matches('L').split_once("-L") else {
        return String::new();
    };
    let (a, b) = (
        a.parse::<usize>().unwrap_or(1),
        b.parse::<usize>().unwrap_or(0),
    );
    all.iter()
        .filter(|(n, _)| *n >= a && *n <= b)
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Faithful conversions for the slots of a whole-file conversion call: the source text is read from the prefix lines
/// the slot names.
pub fn whole_convert_answer(user: &str) -> String {
    let all = prefix_lines(user);
    convert_with(user, &|slot| {
        lines_range(&all, slot["lines"].as_str().unwrap_or(""))
    })
}

fn convert_with(user: &str, region_of: &dyn Fn(&Value) -> String) -> String {
    let qs: Vec<Value> = slots_of(user)
        .iter()
        .map(|slot| {
            let region = region_of(slot);
            let mut lines = region.lines();
            let stem_en = strip_number(lines.next().unwrap_or(""));
            let rest: Vec<&str> = lines.collect();
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let choices = slot["choices"].as_str().unwrap_or("keep");
            let kind = slot["kind"].as_str().unwrap_or("single");
            // the stated answer
            let answer_line = rest.iter().find_map(|l| {
                let t = l.trim();
                t.strip_prefix("Answer:").map(|a| a.trim().to_string())
            });
            let mut options: Vec<(String, bool)> = Vec::new();
            match choices {
                "keep" => {
                    let kept: Vec<(char, String)> = rest
                        .iter()
                        .filter_map(|l| {
                            let mut c = l.chars();
                            let letter = c.next()?;
                            let sep = c.next()?;
                            (letter.is_ascii_uppercase() && (sep == '.' || sep == ')'))
                                .then(|| (letter, c.as_str().trim().to_string()))
                        })
                        .collect();
                    let stated: Vec<char> = answer_line
                        .as_deref()
                        .map(|a| a.chars().filter(|c| c.is_ascii_uppercase()).collect())
                        .unwrap_or_default();
                    for (i, (letter, text)) in kept.iter().enumerate() {
                        let right = if stated.is_empty() {
                            if kind == "multi" { i < 2 } else { i == 0 }
                        } else {
                            stated.contains(letter)
                        };
                        options.push((text.clone(), right));
                    }
                }
                "truefalse" => {
                    let is_true = !answer_line
                        .as_deref()
                        .is_some_and(|a| a.eq_ignore_ascii_case("False"));
                    options.push(("yes".to_string(), is_true));
                    options.push(("no".to_string(), !is_true));
                }
                _ => {
                    let count = slot["count"].as_u64().unwrap_or(4) as usize;
                    for k in 0..count {
                        options.push((format!("{} {}", pw(5000 + n * 10 + k), pw(6000 + n * 10 + k)), k == 0));
                    }
                }
            }
            let opts: Vec<Value> = options
                .iter()
                .enumerate()
                .map(|(k, (en, right))| {
                    json!({"en": en, "zh": han(4000 + n * 10 + k, 3), "ja": kana(4000 + n * 10 + k, 3), "correct": right})
                })
                .collect();
            json!({
                "slot": slot["slot"],
                "stem": {"en": stem_en, "zh": han(100 + n, 9), "ja": kana(100 + n, 9)},
                "options": opts,
                "explanation": tri(2000 + n, 8),
            })
        })
        .collect();
    json!({"questions": qs}).to_string()
}

/// A classification answer by the marker the made-up files carry (`KIND:QUESTIONS`, `KIND:MATERIAL`, `KIND:NONE`).
pub fn classify_answer(user: &str) -> String {
    let kind = if user.contains("KIND:QUESTIONS") {
        "questions"
    } else if user.contains("KIND:MATERIAL") {
        "material"
    } else if user.contains("KIND:NONE") {
        "none"
    } else {
        "material"
    };
    json!({"kind": kind, "language": "en"}).to_string()
}

/// A segmentation answer: every line that starts with `Q<digits>.` opens a question; it ends before the next one.
pub fn segment_answer(user: &str) -> String {
    let (mut from, mut main_end) = (1usize, usize::MAX);
    if let Some(rest) = user.strip_prefix("Report the questions that start on lines L") {
        let a: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        from = a.parse().unwrap_or(1);
        if let Some(i) = rest.find(" to L") {
            let b: String = rest[i + 5..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            main_end = b.parse().unwrap_or(usize::MAX);
        }
    }
    let mut shown: Vec<(usize, String)> = Vec::new();
    for line in user.lines() {
        if let Some(rest) = line.strip_prefix('L') {
            if let Some((n, text)) = rest.split_once(": ") {
                if let Ok(n) = n.parse::<usize>() {
                    shown.push((n, text.to_string()));
                }
            }
        }
    }
    let is_start = |t: &str| {
        t.strip_prefix('Q').is_some_and(|r| {
            r.chars().next().is_some_and(|c| c.is_ascii_digit()) && r.contains('.')
        })
    };
    let starts: Vec<usize> = shown
        .iter()
        .filter(|(_, t)| is_start(t))
        .map(|(n, _)| *n)
        .collect();
    let last = shown.last().map_or(0, |(n, _)| *n);
    let mut out = Vec::new();
    for (i, &st) in starts.iter().enumerate() {
        if st < from || st > main_end {
            continue;
        }
        let end = starts.get(i + 1).map_or(last, |n| n - 1);
        let text = shown
            .iter()
            .find(|(n, _)| *n == st)
            .map(|(_, t)| t.as_str())
            .unwrap_or("");
        let kind = if text.contains("[tf]") {
            "truefalse"
        } else if text.contains("[open]") {
            "open"
        } else {
            "single"
        };
        out.push(json!({"start": st, "end": end, "type": kind}));
    }
    json!({"questions": out}).to_string()
}

/// The recognition answer of the whole-file prompt: the kind by the file's marker, and for a questions file every
/// question's range. A question opens at a line that is not an answer choice, not an answer line and not the marker
/// line; it runs to the line before the next such line (the stub is a model that reads made-up files perfectly).
/// `drop_every`: leaves out every n-th question (a model that misses some); 0 = none.
pub fn whole_recognise_answer(user: &str, drop_every: usize) -> String {
    let all = prefix_lines(user);
    let kind = if user.contains("KIND:QUESTIONS") {
        "questions"
    } else if user.contains("KIND:NONE") {
        "none"
    } else {
        "material"
    };
    if kind != "questions" {
        return json!({"kind": kind, "language": "en", "questions": []}).to_string();
    }
    let is_option = |t: &str| {
        let mut c = t.chars();
        matches!((c.next(), c.next()), (Some('A'..='H'), Some('.'))) || t.starts_with("Answer:")
    };
    let is_stem = |t: &str| t.contains('?') && !is_option(t);
    let starts: Vec<usize> = all
        .iter()
        .filter(|(_, t)| is_stem(t))
        .map(|(n, _)| *n)
        .collect();
    let last = all.last().map_or(0, |(n, _)| *n);
    let mut out = Vec::new();
    for (i, &st) in starts.iter().enumerate() {
        if drop_every > 0 && (i + 1) % drop_every == 0 {
            continue;
        }
        let end = starts.get(i + 1).map_or(last, |n| n - 1);
        let text = all
            .iter()
            .find(|(n, _)| *n == st)
            .map_or("", |(_, t)| t.as_str());
        let kind = if text.contains("[tf]") {
            "truefalse"
        } else if text.contains("[open]") {
            "open"
        } else {
            "single"
        };
        out.push(json!({"start": st, "end": end, "type": kind}));
    }
    json!({"kind": "questions", "language": "en", "questions": out}).to_string()
}

/// A model that answers every prompt of the service correctly.
pub fn good_model() -> Script {
    Arc::new(|system: &str, user: &str| {
        let task = user.rsplit_once("<<<END FILE").map_or(user, |(_, t)| t);
        let text = if system.starts_with("You work on one file of a user of Xexam") {
            if task.contains("TASK RECOGNISE") {
                whole_recognise_answer(user, 0)
            } else if task.contains("TASK CONVERT") {
                whole_convert_answer(user)
            } else {
                generation_answer(user)
            }
        } else if system.starts_with("You sort a user's file") {
            classify_answer(user)
        } else if system.starts_with("You find the exam questions") {
            segment_answer(user)
        } else if system.starts_with("You convert exam questions") {
            convert_answer(user)
        } else if system.starts_with("You write new multiple-choice exam questions")
            || system.starts_with("You write exam questions from a user's study material")
        {
            generation_answer(user)
        } else {
            return Reply::Fall;
        };
        Reply::Text(text)
    })
}

/// A made-up questions file (one paragraph per line) with `n` multiple-choice questions, one without options, one
/// true / false. `KIND:QUESTIONS` marks it for the scripted classifier.
pub fn questions_lines(n: usize) -> Vec<String> {
    let mut v = vec!["KIND:QUESTIONS course notes".to_string()];
    for i in 1..=n {
        v.push(format!(
            "Q{i}. Which kitchen device {} keeps a stock pot at a steady simmer?",
            en_text(300 + i, 3)
        ));
        v.push(format!("A. {} {}", pw(500 + i), pw(600 + i)));
        v.push(format!("B. {} {}", pw(700 + i), pw(800 + i)));
        v.push(format!("C. {} {}", pw(900 + i), pw(1000 + i)));
        v.push(format!("D. {} {}", pw(1100 + i), pw(1200 + i)));
        v.push("Answer: B".to_string());
    }
    v
}

/// A made-up study text for the material mode.
pub fn material_lines(paragraphs: usize) -> Vec<String> {
    let mut v = vec!["KIND:MATERIAL kitchen handbook".to_string()];
    for i in 0..paragraphs {
        v.push(format!(
            "Section {i}: the {} device holds a stock pot at a steady simmer by pausing the burner when the {} probe reads hot. {}",
            pw(40 + i),
            pw(70 + i),
            en_text(2000 + i, 40)
        ));
    }
    v
}

/// A made-up file of 70 UNNUMBERED questions in the shape of A-OWNER-70: each stem is its own paragraph, 57 have the
/// options A to D, 8 (spread over the file) have A to E, 3 have an option without its letter and 2 carry two options on
/// one line (those five sit after the 50th question, so a 50-question import converts only clean ones). No answer
/// lines. Everything is invented (a fictional kitchen-equipment course).
pub fn unnumbered_lines() -> Vec<String> {
    const FIVE: [usize; 8] = [4, 12, 20, 28, 36, 44, 52, 60];
    const NO_LETTER: [usize; 3] = [53, 61, 67];
    const TWO_ON_A_LINE: [usize; 2] = [56, 69];
    let mut v = vec!["KIND:QUESTIONS kitchen course workbook".to_string()];
    for i in 0..70usize {
        v.push(format!(
            "In the {} kitchen, which device {} keeps a stock pot at a steady simmer?",
            pw(100_000 + i),
            en_text(3000 + i, 3)
        ));
        let letters: &[&str] = if FIVE.contains(&i) {
            &["A", "B", "C", "D", "E"]
        } else {
            &["A", "B", "C", "D"]
        };
        for (k, l) in letters.iter().enumerate() {
            let text = format!("{} {}", pw(10_000 + i * 10 + k), pw(20_000 + i * 10 + k));
            if NO_LETTER.contains(&i) && *l == "C" {
                v.push(text);
            } else if TWO_ON_A_LINE.contains(&i) && *l == "C" {
                v.push(format!("C. {text} D. {}", pw(30_000 + i)));
            } else if TWO_ON_A_LINE.contains(&i) && *l == "D" {
                continue;
            } else {
                v.push(format!("{l}. {text}"));
            }
        }
    }
    v
}
