//! Question layer: pure string functions, no XML dependency. Maps one-to-one to lines 111-358 of `app/xlsx_parser.py`.
//!
//! Conventions: every `strip()` uses Python's whitespace set (`py_strip`); all lengths count code points; every `\s` in a regex is written as
//! `[\s\x1c-\x1f]` (Python's `\s` also matches U+001C..U+001F, which Rust's does not), and every `\d` uses Python's Nd table.
use super::id::make_id;
use super::py_space::{
    char_len, digit_class_inner, digit_value, digits_to_canonical, py_strip, take_chars,
};
use crate::error::ImportError;
use crate::models::{Answer, En, Explain, ExplainText, Item, Kind, Opt, Question, Zh};
use regex::{NoExpand, Regex};
use std::collections::HashMap;
use std::sync::LazyLock;

/// Fragment: the inside of a character class for Python's `\s`.
const S: &str = r"\s\x1c-\x1f";

#[allow(clippy::expect_used)]
fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regex")
}

struct Res {
    heading: Regex,
    opt_line: Regex,
    num_prefix: Regex,
    hint: Regex,
    hint_words: Regex,
    split_slash: Regex,
    paren: Regex,
    item_split: Regex,
    item_seg: Regex,
    item_fallback: Regex,
    letters_full: Regex,
    canon: Regex,
    hot_lead: Regex,
    ans_line: Regex,
    note: Regex,
}

static RES: LazyLock<Res> = LazyLock::new(|| {
    let d = digit_class_inner();
    Res {
        heading: re(r"【([^】\n]{1,24})】"),
        opt_line: re(&format!(
            r"^[{S}]*([A-Ha-h])(?:[{S}]*[.．、)）:：][{S}]*(.*)|[{S}]*)$"
        )),
        num_prefix: re(&format!(r"^[{S}]*([{d}]{{1,2}})[{S}]*[.．、)）][{S}]*")),
        hint: re(r"[\t\x{3000} ]+([^\t\x{3000}/]+?(?: +/ +[^\t\x{3000}/]+?)+)[ \t\x{3000}]*$"),
        // English items separated from their choices by plain spaces only: single-word choices ("… is wet. Yes / No")
        hint_words: re(r" +([^\s/]+(?: +/ +[^\s/]+)+) *$"),
        split_slash: re(&format!(r"[{S}]+/[{S}]+")),
        paren: re(r"[（(][^）)]*[）)]"),
        item_split: re(r"[；;\n，,]+"),
        item_seg: re(&format!(
            r"^[{S}]*([{d}]+)[{S}]*[.．、:：)）\-]*[{S}]*(.+?)[{S}]*$"
        )),
        item_fallback: re(&format!(
            r"([{d}]+)[{S}]*[.．、:：)）]?[{S}]*([^{S}{d};；,，]+)"
        )),
        letters_full: re(r"^[A-Ha-h]{2,6}$"),
        canon: re(&format!(r"[{S}_\-–—‐・.．、,，/／]+")),
        hot_lead: re(&format!(
            r"(?i)^[{S}]*(?:热点题|熱點題|热点|hotspot)[{S}]*[:：][{S}]*"
        )),
        ans_line: re(r"(?i)(?:^|\n)[ \t]*(?:答案|正解|正确答案|Answer|Ans)[ \t]*[:：=][ \t]*(.+)"),
        note: re(r"[（(]([^）)]*)[）)]"),
    }
});

// ------------------------------------------------------------------ Headings / sections

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    Answer,
    Explanation,
    Mnemonic,
    Translation,
}

/// `classify_heading`: substring match, priority answer > explanation > mnemonic > translation.
pub fn classify_heading(name: &str) -> Option<Heading> {
    if name.contains("答案") || name.contains("正解") {
        Some(Heading::Answer)
    } else if name.contains("解析") || name.contains("解説") {
        Some(Heading::Explanation)
    } else if name.contains("口诀")
        || name.contains("口訣")
        || name.contains("记忆")
        || name.contains("記憶")
    {
        Some(Heading::Mnemonic)
    } else if name.contains("翻译") || name.contains("翻訳") || name.contains("译文") {
        Some(Heading::Translation)
    } else {
        None
    }
}

#[derive(Debug, Default, Clone)]
pub struct Sections {
    pub answer: Option<String>,
    pub explanation: Option<String>,
    pub mnemonic: Option<String>,
    pub translation: Option<String>,
}

/// `split_sections`: only the first occurrence of each kind is kept; a section's text runs to the next recognized heading or the end, then is stripped.
pub fn split_sections(text: &str) -> Sections {
    split_sections_with(text, classify_heading)
}

/// [`split_sections`] with another heading classifier (the six-column layout also accepts English headings).
fn split_sections_with(text: &str, classify: fn(&str) -> Option<Heading>) -> Sections {
    let mut marks: Vec<(usize, usize, Heading)> = Vec::new();
    for m in RES.heading.captures_iter(text) {
        let (Some(whole), Some(name)) = (m.get(0), m.get(1)) else {
            continue;
        };
        if let Some(kind) = classify(name.as_str()) {
            marks.push((whole.start(), whole.end(), kind));
        }
    }
    let mut secs = Sections::default();
    for (i, (_, e, kind)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map_or(text.len(), |n| n.0);
        let seg = py_strip(text.get(*e..end).unwrap_or("")).to_string();
        let slot = match kind {
            Heading::Answer => &mut secs.answer,
            Heading::Explanation => &mut secs.explanation,
            Heading::Mnemonic => &mut secs.mnemonic,
            Heading::Translation => &mut secs.translation,
        };
        if slot.is_none() {
            *slot = Some(seg);
        }
    }
    secs
}

// ------------------------------------------------------------------ Options / hotspot

/// `OPT_LINE.match(ln)`: returns (uppercase letter, text of group 2; empty string if absent).
fn opt_line(ln: &str) -> Option<(u8, String)> {
    let caps = RES.opt_line.captures(ln)?;
    let letter = caps.get(1)?.as_str().bytes().next()?.to_ascii_uppercase();
    let rest = caps.get(2).map_or("", |m| m.as_str()).to_string();
    Some((letter, rest))
}

/// Multiple-choice question: supports "A. text" and a letter alone on its own line followed by the text on the next line. -> (stem, [options])
pub fn parse_options(body: &str) -> (String, Vec<Opt>) {
    let lines: Vec<&str> = body.split('\n').collect();
    let start = lines
        .iter()
        .position(|ln| opt_line(ln).is_some_and(|(k, _)| k == b'A'));
    let Some(start) = start else {
        return (py_strip(body).to_string(), Vec::new());
    };
    let mut opts: Vec<(u8, Vec<String>)> = Vec::new();
    let mut expected = b'A';
    for ln in lines.iter().skip(start) {
        let m = opt_line(ln);
        if let Some((k, rest)) = m.filter(|(k, _)| *k == expected) {
            opts.push((k, vec![py_strip(&rest).to_string()]));
            expected = expected.saturating_add(1);
        } else if let Some(cur) = opts.last_mut() {
            cur.1.push(py_strip(ln).to_string());
        }
    }
    let options: Vec<Opt> = opts
        .into_iter()
        .map(|(k, ls)| {
            let joined = ls
                .iter()
                .filter(|x| !x.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            Opt {
                key: char::from(k).to_string(),
                text: py_strip(&joined).to_string(),
            }
        })
        .collect();
    if options.len() < 2 || options.iter().any(|o| o.text.is_empty()) {
        return (py_strip(body).to_string(), Vec::new());
    }
    let stem = lines
        .iter()
        .take(start)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    (py_strip(&stem).to_string(), options)
}

/// `NUM_PREFIX.sub('', ln, count=1)`: returns the text with the number prefix removed.
fn strip_num_prefix(ln: &str) -> &str {
    match RES.num_prefix.find(ln) {
        Some(m) => ln.get(m.end()..).unwrap_or(ln),
        None => ln,
    }
}

/// Hotspot question: each sub-item ends with a choice such as "はい / いいえ". -> (stem, [sub-items])
pub fn parse_hotspot(body: &str) -> (String, Vec<Item>) {
    hotspot_items(body, false)
}

/// [`parse_hotspot`] for the English base text of the six-column layout. English sentences contain spaces, so an item whose
/// choices follow it after a plain space ("1. Water is wet. Yes / No") would be split at its first space by the rule of the
/// older layouts (made for Japanese, which has no spaces inside a sentence). Here a line with a tab or an ideographic space
/// is split as before (at the last one); a line with plain spaces only takes the single-word choices at its end. Choices of
/// several words ("Model Stage / Post-Processing") therefore need a tab or an ideographic space before them.
fn parse_hotspot_en(body: &str) -> (String, Vec<Item>) {
    hotspot_items(body, true)
}

fn hotspot_items(body: &str, english: bool) -> (String, Vec<Item>) {
    let mut stem_lines: Vec<&str> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut common: Option<Vec<String>> = None;
    for ln in body.split('\n') {
        if py_strip(ln).is_empty() {
            if items.is_empty() {
                stem_lines.push(ln);
            }
            continue;
        }
        let text = strip_num_prefix(ln);
        let had_num = text != ln;
        let hint = if english && !text.contains(['\t', '\u{3000}']) {
            &RES.hint_words
        } else {
            &RES.hint
        };
        if let Some(caps) = hint.captures(text) {
            if let (Some(whole), Some(g1)) = (caps.get(0), caps.get(1)) {
                let choices: Vec<String> = RES
                    .split_slash
                    .split(g1.as_str())
                    .map(|t| py_strip(t).to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
                if choices.len() >= 2 {
                    if common.is_none() {
                        common = Some(choices.clone());
                    }
                    let head = py_strip(text.get(..whole.start()).unwrap_or("")).to_string();
                    pending.push(head);
                    items.push(Item {
                        text: py_strip(&pending.join("\n")).to_string(),
                        choices,
                    });
                    pending.clear();
                    continue;
                }
            }
        }
        if let (false, true, Some(cm)) = (items.is_empty(), had_num, common.as_ref()) {
            pending.push(py_strip(text).to_string());
            items.push(Item {
                text: py_strip(&pending.join("\n")).to_string(),
                choices: cm.clone(),
            });
            pending.clear();
        } else if !items.is_empty() {
            pending.push(py_strip(ln).to_string());
        } else {
            stem_lines.push(ln);
        }
    }
    (py_strip(&stem_lines.join("\n")).to_string(), items)
}

// ------------------------------------------------------------------ Answers

/// `parse_letters`: each bracketed span is replaced wholesale with a space; only the first non-empty line is examined; finds A-H/a-h with no ASCII letter or digit on either side.
pub fn parse_letters(ans: &str) -> Vec<String> {
    let s = RES.paren.replace_all(ans, NoExpand(" "));
    let first = s
        .split('\n')
        .find(|ln| !py_strip(ln).is_empty())
        .unwrap_or("");
    let chars: Vec<char> = first.chars().collect();
    let mut letters: Vec<String> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        if !matches!(c, 'A'..='H' | 'a'..='h') {
            continue;
        }
        let before_ok = i
            .checked_sub(1)
            .and_then(|j| chars.get(j))
            .is_none_or(|p| !p.is_ascii_alphanumeric());
        let after_ok = chars.get(i + 1).is_none_or(|n| !n.is_ascii_alphanumeric());
        if before_ok && after_ok {
            let k = c.to_ascii_uppercase().to_string();
            if !letters.contains(&k) {
                letters.push(k);
            }
        }
    }
    if letters.is_empty() && RES.letters_full.is_match(py_strip(first)) {
        let mut set: Vec<String> = py_strip(first)
            .chars()
            .map(|c| c.to_ascii_uppercase().to_string())
            .collect();
        set.sort();
        set.dedup();
        letters = set;
    }
    letters.sort();
    letters
}

/// Sub-item answers: number (canonical decimal string) -> answer text. Returns Err if a number exceeds 4300 digits (Python raises ValueError there).
pub fn parse_item_answers(ans: &str) -> Result<HashMap<String, String>, ()> {
    let s = RES.paren.replace_all(ans, NoExpand(""));
    let mut out: HashMap<String, String> = HashMap::new();
    for seg in RES.item_split.split(&s) {
        if let Some(caps) = RES.item_seg.captures(seg) {
            if let (Some(n), Some(v)) = (caps.get(1), caps.get(2)) {
                let key = digits_to_canonical(n.as_str())?.ok_or(())?;
                out.insert(key, v.as_str().to_string());
            }
        }
    }
    if out.len() < 2 {
        let mut fb: HashMap<String, String> = HashMap::new();
        for caps in RES.item_fallback.captures_iter(&s) {
            if let (Some(n), Some(v)) = (caps.get(1), caps.get(2)) {
                let key = digits_to_canonical(n.as_str())?.ok_or(())?;
                fb.insert(key, v.as_str().to_string());
            }
        }
        out = fb;
    }
    Ok(out)
}

fn is_yes(t: &str) -> bool {
    matches!(
        t,
        "是" | "yes" | "y" | "true" | "はい" | "○" | "〇" | "对" | "對" | "正确" | "正確"
    )
}

fn is_no(t: &str) -> bool {
    matches!(
        t,
        "否" | "no" | "n" | "false" | "いいえ" | "×" | "✕" | "错" | "錯" | "错误" | "不正確"
    )
}

/// `canon`: strip surrounding whitespace, lowercase, delete separators; the various spellings of yes / no are normalized.
pub fn canon(s: &str) -> String {
    let lowered = py_strip(s).to_lowercase();
    let t = RES.canon.replace_all(&lowered, NoExpand("")).into_owned();
    if is_yes(&t) {
        "\u{0}yes".to_string()
    } else if is_no(&t) {
        "\u{0}no".to_string()
    } else {
        t
    }
}

fn find_choice(choices: &[String], val: &str) -> Option<usize> {
    let cv = canon(val);
    choices.iter().position(|ch| canon(ch) == cv)
}

// ------------------------------------------------------------------ Translation

/// `_norm_nl`
pub fn norm_nl(s: &str) -> String {
    py_strip(&s.replace("\r\n", "\n").replace('\r', "\n")).to_string()
}

/// Choices at the end of a hotspot sub-item: "…。　是 / 否". -> (text without the choices, choices). Returns the text unchanged if there are no choices.
fn split_trailing_choices(text: &str) -> (String, Option<Vec<String>>) {
    let last = text.rsplit('\n').next().unwrap_or(text);
    if let Some(caps) = RES.hint.captures(last) {
        if let (Some(whole), Some(g1)) = (caps.get(0), caps.get(1)) {
            let choices: Vec<String> = RES
                .split_slash
                .split(g1.as_str())
                .map(|t| py_strip(t).to_string())
                .filter(|t| !t.is_empty())
                .collect();
            if choices.len() >= 2 {
                let head_last = py_strip(last.get(..whole.start()).unwrap_or(""));
                let cut = text.len() - last.len();
                let mut head = text.get(..cut).unwrap_or("").to_string();
                head.push_str(head_last);
                return (py_strip(&head).to_string(), Some(choices));
            }
        }
    }
    (text.to_string(), None)
}

/// The language's own choices for each sub-item. Used only if **every sub-item has them and the count matches the Japanese** (matched one by one, no guessing); otherwise the whole question gets none,
/// and the UI falls back to showing the Japanese choices. A choice's position is the answer index, so as long as the counts match, "the j-th" means the same thing in all three languages.
fn localized_choices(
    split: &[(String, Option<Vec<String>>)],
    ja_items: &[Item],
) -> Option<Vec<Vec<String>>> {
    if split.len() != ja_items.len() {
        return None;
    }
    let mut out = Vec::with_capacity(split.len());
    for ((_, ch), ja) in split.iter().zip(ja_items) {
        let ch = ch.as_ref()?;
        if ch.len() != ja.choices.len() {
            return None;
        }
        out.push(ch.clone());
    }
    Some(out)
}

/// Split the column B translation into the same structure as the Japanese question; returns None if they do not line up (no Chinese toggle is offered).
pub fn parse_translation(tr: &str, options: &[Opt], items: &[Item]) -> Option<Zh> {
    let normed = norm_nl(tr);
    let stripped = RES.hot_lead.replace(&normed, NoExpand(""));
    let tr = py_strip(&stripped);
    if tr.is_empty() {
        return None;
    }
    if !options.is_empty() {
        let (stem, opts) = parse_options(tr);
        let same =
            opts.len() == options.len() && opts.iter().zip(options).all(|(a, b)| a.key == b.key);
        if !same {
            return None;
        }
        return Some(Zh {
            stem,
            options: Some(opts.into_iter().map(|o| o.text).collect()),
            items: None,
            choices: None,
        });
    }
    if !items.is_empty() {
        let mut stem_lines: Vec<&str> = Vec::new();
        let mut parts: Vec<Vec<String>> = Vec::new();
        for ln in tr.split('\n') {
            let num = RES.num_prefix.captures(ln).and_then(|caps| {
                let whole = caps.get(0)?;
                let g1 = caps.get(1)?;
                let mut v: usize = 0;
                for c in g1.as_str().chars() {
                    v = v * 10 + usize::from(digit_value(c)?);
                }
                Some((v, whole.end()))
            });
            if let Some((v, end)) = num.filter(|(v, _)| *v == parts.len() + 1) {
                let _ = v;
                parts.push(vec![py_strip(ln.get(end..).unwrap_or("")).to_string()]);
            } else if let Some(cur) = parts.last_mut() {
                if !py_strip(ln).is_empty() {
                    cur.push(py_strip(ln).to_string());
                }
            } else {
                stem_lines.push(ln);
            }
        }
        if parts.len() != items.len() {
            return None;
        }
        // If a sub-item ends with its own language's choices ("…。　是 / 否"), split them off; otherwise leave it as it was (choices = None)
        let split: Vec<(String, Option<Vec<String>>)> = parts
            .iter()
            .map(|p| split_trailing_choices(py_strip(&p.join("\n"))))
            .collect();
        let choices = localized_choices(&split, items);
        return Some(Zh {
            stem: py_strip(&stem_lines.join("\n")).to_string(),
            options: None,
            items: Some(split.into_iter().map(|(t, _)| t).collect()),
            choices,
        });
    }
    None
}

// ------------------------------------------------------------------ Whole question

pub const SKIP_NO_STRUCTURE: &str = "没有找到选项（A、B、C…）或带「はい / いいえ」的小项";
pub const SKIP_NO_ANSWER: &str = "没有找到答案（C 列要写「【答案】…」）";

/// Result of `parse_question`: success, unreadable (with a reason), or the whole file must be rejected.
#[derive(Debug)]
pub enum QuestionOutcome {
    Ok(Box<Question>),
    Skip(String),
    Fatal(ImportError),
}

/// Legacy three-column format (A Japanese, B Chinese, C answer). Now only unit tests use it directly; workbook parsing goes through `parse_question_with_english`.
#[cfg(test)]
pub fn parse_question(sheet: &str, row: i64, a: &str, b: &str, c: &str) -> QuestionOutcome {
    parse_question_with_english(sheet, row, a, b, c, "")
}

/// Splits the English version into the same structure as the Japanese: single / multiple choice carries `options`, a hotspot question carries `items` and the English choices of each sub-item.
/// Returns None if the option letters or sub-item count do not match the Japanese (the page then offers no English toggle, rather than showing misaligned content).
pub fn parse_english(en: &str, options: &[Opt], items: &[Item]) -> Option<En> {
    let normed = norm_nl(en);
    let stripped = RES.hot_lead.replace(&normed, NoExpand(""));
    let text = py_strip(&stripped);
    let text = text.strip_prefix("HOTSPOT").map(py_strip).unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    if !options.is_empty() {
        let (stem, opts) = parse_options(text);
        let same =
            opts.len() == options.len() && opts.iter().zip(options).all(|(a, b)| a.key == b.key);
        if !same || stem.is_empty() {
            return None;
        }
        return Some(En {
            stem,
            options: Some(opts.into_iter().map(|o| o.text).collect()),
            items: None,
            choices: None,
        });
    }
    if !items.is_empty() {
        let (stem, en_items) = parse_hotspot(text);
        if en_items.len() != items.len() || stem.is_empty() {
            return None;
        }
        let split: Vec<(String, Option<Vec<String>>)> = en_items
            .into_iter()
            .map(|i| (i.text, Some(i.choices)))
            .collect();
        let choices = localized_choices(&split, items);
        return Some(En {
            stem,
            options: None,
            items: Some(split.into_iter().map(|(t, _)| t).collect()),
            choices,
        });
    }
    None
}

/// The text that goes into the question id next to the stem: the first option, or the first hotspot item.
fn first_text_of(options: &[Opt], items: &[Item]) -> String {
    match options.first() {
        Some(o) => o.text.clone(),
        None => items.first().map(|i| i.text.clone()).unwrap_or_default(),
    }
}

/// The notes written in parentheses inside an answer ("B (confidence: high)", "B（把握度：高）"), joined with "；".
/// The notes are never part of the answer itself: `parse_letters` and `parse_item_answers` drop every bracketed span.
fn answer_note(ans: &str) -> String {
    RES.note
        .captures_iter(ans)
        .filter_map(|caps| caps.get(1))
        .map(|m| py_strip(m.as_str()))
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("；")
}

/// Turns the answer text into the answer of a question whose structure is `options` (single / multiple choice) or
/// `items` (hotspot). `other_langs` are the choices written in the other language columns, item by item (only columns
/// that line up with the base question): a hotspot answer may be written with the choice words of any of them, because
/// the j-th choice is the same choice in every language. `Err` carries the outcome to return for this row as is
/// (unreadable with a reason, or a fatal error for the whole file).
fn resolve_answer(
    ans: &str,
    options: &[Opt],
    items: &[Item],
    other_langs: &[&Vec<Vec<String>>],
) -> Result<(Kind, Answer), QuestionOutcome> {
    if !options.is_empty() {
        let letters = parse_letters(ans);
        let keys: Vec<&str> = options.iter().map(|o| o.key.as_str()).collect();
        if letters.is_empty() {
            return Err(QuestionOutcome::Skip(format!(
                "答案「{}」里没有找到选项字母",
                take_chars(ans, 20)
            )));
        }
        let bad: Vec<&str> = letters
            .iter()
            .map(String::as_str)
            .filter(|x| !keys.contains(x))
            .collect();
        if !bad.is_empty() {
            return Err(QuestionOutcome::Skip(format!(
                "答案 {} 不在选项 {} 里",
                bad.join("、"),
                keys.join("、")
            )));
        }
        let kind = if letters.len() > 1 {
            Kind::Multi
        } else {
            Kind::Single
        };
        return Ok((kind, Answer::Letters(letters)));
    }
    let Ok(vals) = parse_item_answers(ans) else {
        return Err(QuestionOutcome::Fatal(ImportError::Message(
            "答案里有超长的数字（超过 4300 位），已拒绝。".to_string(),
        )));
    };
    let mut idx: Vec<i64> = Vec::new();
    for (k0, it) in items.iter().enumerate() {
        let k = k0 + 1;
        let Some(v) = vals.get(&k.to_string()) else {
            return Err(QuestionOutcome::Skip(format!("第 {k} 小项没有答案")));
        };
        // Look in the base question's own choices first, then in the choices written in the other columns in turn
        let Some(j) = find_choice(&it.choices, v).or_else(|| {
            other_langs
                .iter()
                .find_map(|c| c.get(k0).and_then(|ch| find_choice(ch, v)))
        }) else {
            return Err(QuestionOutcome::Skip(format!(
                "第 {k} 小项的答案「{v}」不在选项「{}」里",
                it.choices.join(" / ")
            )));
        };
        idx.push(i64::try_from(j).unwrap_or(0));
    }
    let mut extra: Vec<&String> = vals
        .keys()
        // A key is within 1..=N iff it is exactly the decimal form of some k (no leading zeros or plus sign); an O(1) check, instead of scanning all N items for every key
        .filter(|key| {
            !key.parse::<usize>()
                .ok()
                .is_some_and(|k| (1..=items.len()).contains(&k) && k.to_string() == **key)
        })
        .collect();
    extra.sort_by(|x, y| (x.len(), x.as_str()).cmp(&(y.len(), y.as_str())));
    if !extra.is_empty() {
        let joined = extra
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("、");
        return Err(QuestionOutcome::Skip(format!(
            "答案里有第 {joined} 项，但题目只有 {} 个小项",
            items.len()
        )));
    }
    Ok((Kind::Hotspot, Answer::Indexes(idx)))
}

/// In the four-column format column A is English: when `english` is non-empty, `en` is also split out. The rest of the logic is identical to the three-column format (the question id still comes from the Japanese).
pub fn parse_question_with_english(
    sheet: &str,
    row: i64,
    a: &str,
    b: &str,
    c: &str,
    english: &str,
) -> QuestionOutcome {
    let text = norm_nl(a);
    let b = norm_nl(b);
    let c = norm_nl(c);
    let combined = py_strip(&format!("{b}\n\n{c}")).to_string();
    let secs = split_sections(&combined);

    let first_line = text
        .split('\n')
        .map(py_strip)
        .find(|ln| !ln.is_empty())
        .unwrap_or("");
    let lower_first = first_line.to_lowercase();
    let is_hot = lower_first == "ホットスポット" || lower_first == "hotspot";
    let body: &str = if is_hot {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };

    let mut stem = String::new();
    let mut options: Vec<Opt> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    if !is_hot {
        (stem, options) = parse_options(body);
    }
    if options.is_empty() {
        (stem, items) = parse_hotspot(body);
    }
    if options.is_empty() && items.is_empty() {
        return QuestionOutcome::Skip(SKIP_NO_STRUCTURE.to_string());
    }

    let mut ans: Option<String> = secs.answer.clone();
    if ans.is_none() {
        if let Some(m) = RES
            .ans_line
            .captures(&combined)
            .and_then(|caps| caps.get(1))
        {
            ans = Some(py_strip(m.as_str()).to_string());
        }
    }
    if ans.is_none() {
        for cand in [&c, &b] {
            if !cand.is_empty()
                && !cand.contains('\n')
                && char_len(cand) <= 40
                && !cand.starts_with('【')
            {
                ans = Some(cand.clone());
                break;
            }
        }
    }
    let ans = match ans {
        Some(a) if !a.is_empty() => a,
        _ => return QuestionOutcome::Skip(SKIP_NO_ANSWER.to_string()),
    };

    // Split the Chinese / English text first: a hotspot answer may be written with the choice words used in these two columns (see below)
    let mut translation = secs.translation.clone();
    if translation.is_none() && !b.is_empty() && !b.contains('【') && char_len(&b) > 40 {
        translation = Some(b.clone());
    }
    let translation_text = translation.clone().unwrap_or_default();
    let zh = if translation_text.is_empty() {
        None
    } else {
        parse_translation(&translation_text, &options, &items)
    };
    let english_text = py_strip(&norm_nl(english)).to_string();
    let en = if english_text.is_empty() {
        None
    } else {
        parse_english(&english_text, &options, &items)
    };

    let other_langs: Vec<&Vec<Vec<String>>> = [zh.as_ref(), en.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|z| z.choices.as_ref())
        .collect();
    let (kind, answer) = match resolve_answer(&ans, &options, &items, &other_langs) {
        Ok(found) => found,
        Err(outcome) => return outcome,
    };
    let first_text = first_text_of(&options, &items);
    let note = answer_note(&ans);
    QuestionOutcome::Ok(Box::new(Question {
        kind,
        tag: if is_hot {
            "ホットスポット".to_string()
        } else {
            String::new()
        },
        id: make_id(sheet, &stem, &first_text),
        stem,
        options,
        items,
        answer,
        translation: translation_text,
        zh,
        english: english_text,
        en,
        // Japanese is the base language of the 3 / 4-column layouts: no `lang`, `ja` or `explain` (byte-identical JSON)
        lang: String::new(),
        ja: None,
        explanation: secs.explanation.unwrap_or_default(),
        mnemonic: secs.mnemonic.unwrap_or_default(),
        note,
        explain: None,
        sheet: sheet.to_string(),
        row,
    }))
}

// ------------------------------------------------------------------ Six-column layout (v0.7)
//
// A English question, B Chinese question, C Japanese question, D English answer + explanation + memory aid,
// E the same in Chinese, F the same in Japanese. English is authoritative (owner decision): the question type,
// the options / hotspot items, the question id and the answer key all come from A and D. B and C are aligned item
// by item against the English question; E and F give their language's explanation, and their answers are compared
// with D by meaning, which can produce a preview warning but never makes a row unreadable.

/// A six-column row whose column A is empty.
pub const SKIP6_NO_ENGLISH: &str = "这一行没有英文题目（六列格式以 A 列的英文题目为准）";
/// A six-column row whose English question has neither options nor hotspot items.
pub const SKIP6_NO_STRUCTURE: &str =
    "A 列的英文题目里没有找到选项（A、B、C…）或带「Yes / No」等选项的小项";
/// A six-column row with no answer in column D.
pub const SKIP6_NO_ANSWER: &str = "D 列没有找到英文答案（D 列要写「【Answer】…」）";
/// Preview warning: the answer in column E means something else than the answer in column D.
pub const WARN6_ZH_DIFFERS: &str = "E 列的中文答案和 D 列的英文答案不一致（已按英文答案导入）";
/// Preview warning: the answer in column F means something else than the answer in column D.
pub const WARN6_JA_DIFFERS: &str = "F 列的日文答案和 D 列的英文答案不一致（已按英文答案导入）";

/// Section headings of the six-column layout: every heading of [`classify_heading`] plus the English headings
/// 【Answer】 【Explanation】 【Memory aid】 / 【Mnemonic】 【Translation】 and the Japanese 【答え】 【覚え方】.
/// English names match case-insensitively as substrings, like the legacy words; "explanation" is checked before
/// "answer", so a heading such as 【Answer explanation】 counts as the explanation.
pub fn classify_heading_any(name: &str) -> Option<Heading> {
    if let Some(kind) = classify_heading(name) {
        return Some(kind);
    }
    let lower = name.to_lowercase();
    if lower.contains("explanation") {
        Some(Heading::Explanation)
    } else if lower.contains("memory aid") || lower.contains("mnemonic") || name.contains("覚え方")
    {
        Some(Heading::Mnemonic)
    } else if lower.contains("answer") || name.contains("答え") {
        Some(Heading::Answer)
    } else if lower.contains("translation") {
        Some(Heading::Translation)
    } else {
        None
    }
}

/// Does this cell carry an answer heading in any language (【Answer】, 【答案】, 【正解】, 【答え】, ...)?
/// Used to detect the six-column layout and to tell question rows from header / note rows.
pub fn has_answer_heading(text: &str) -> bool {
    RES.heading.captures_iter(text).any(|m| {
        m.get(1)
            .is_some_and(|name| classify_heading_any(name.as_str()) == Some(Heading::Answer))
    })
}

/// Is this line the marker that opens a hotspot question ("HOTSPOT", "ホットスポット", "热点题", any case)?
fn is_hotspot_marker(line: &str) -> bool {
    let lower = py_strip(line).to_lowercase();
    matches!(
        lower.as_str(),
        "hotspot" | "ホットスポット" | "热点题" | "熱點題" | "热点" | "熱點"
    )
}

/// The answer written in one answer column: the 【Answer】 section, else an "Answer: …" line, else the whole cell when
/// it is a single short line that is not a heading (the same fallbacks as the older layouts).
fn column_answer(text: &str, secs: &Sections) -> Option<String> {
    if let Some(a) = secs.answer.as_deref().filter(|a| !a.is_empty()) {
        return Some(a.to_string());
    }
    if let Some(m) = RES.ans_line.captures(text).and_then(|caps| caps.get(1)) {
        let a = py_strip(m.as_str());
        if !a.is_empty() {
            return Some(a.to_string());
        }
    }
    if !text.is_empty() && !text.contains('\n') && char_len(text) <= 40 && !text.starts_with('【')
    {
        return Some(text.to_string());
    }
    None
}

/// The question text of a Chinese (B) or Japanese (C) column: the text under a 【中文翻译】 / 【翻訳】-like heading when
/// there is one, else the whole cell.
fn question_text_of(cell: &str) -> String {
    let secs = split_sections_with(cell, classify_heading_any);
    match secs.translation {
        Some(t) if !t.is_empty() => t,
        _ => cell.to_string(),
    }
}

/// Splits a Chinese or Japanese question into the structure of the English base question (same option letters, or the
/// same number of hotspot items). Returns None when it does not line up: that language is then greyed out on the page.
/// Hotspot items are read with their trailing choices first ("…　はい / いいえ"); when that does not give the right
/// number of items, items are read by their numbers ("1. …", "2. …") like the Chinese translation of the older layouts.
fn parse_aligned(text: &str, options: &[Opt], items: &[Item]) -> Option<Zh> {
    let normed = norm_nl(text);
    let stripped = RES.hot_lead.replace(&normed, NoExpand(""));
    let mut body = py_strip(&stripped);
    if let Some((first, rest)) = body.split_once('\n') {
        if is_hotspot_marker(first) {
            body = py_strip(rest);
        }
    }
    if body.is_empty() || is_hotspot_marker(body) {
        return None;
    }
    if !options.is_empty() {
        let (stem, opts) = parse_options(body);
        let same =
            opts.len() == options.len() && opts.iter().zip(options).all(|(a, b)| a.key == b.key);
        if !same {
            return None;
        }
        return Some(Zh {
            stem,
            options: Some(opts.into_iter().map(|o| o.text).collect()),
            items: None,
            choices: None,
        });
    }
    if items.is_empty() {
        return None;
    }
    let (stem, own) = parse_hotspot(body);
    if own.len() == items.len() {
        let split: Vec<(String, Option<Vec<String>>)> =
            own.into_iter().map(|i| (i.text, Some(i.choices))).collect();
        let choices = localized_choices(&split, items);
        return Some(Zh {
            stem,
            options: None,
            items: Some(split.into_iter().map(|(t, _)| t).collect()),
            choices,
        });
    }
    parse_translation(body, options, items)
}

/// Does the answer written in another language's column mean the same as the English answer?
///
/// Choice questions compare the option letters. Hotspot questions compare, item by item, the index of the chosen
/// choice: the value is looked up in that language's own choices of the item (`own`, from column B or C when it lines
/// up), then in the English choices; yes / no words of any language are equal to each other (see [`canon`]), so
/// "是", "はい" and "Yes" all pick the same choice. An answer that cannot be mapped at all counts as different.
fn same_answer(
    ans: &str,
    english: &Answer,
    items: &[Item],
    own: Option<&Vec<Vec<String>>>,
) -> bool {
    match english {
        Answer::Letters(letters) => {
            let theirs = parse_letters(ans);
            !theirs.is_empty() && &theirs == letters
        }
        Answer::Indexes(idx) => {
            let mapped = |s: &str| -> Option<bool> {
                let vals = parse_item_answers(s).ok()?;
                if vals.len() != items.len() {
                    return None;
                }
                let mut same = true;
                for (k0, it) in items.iter().enumerate() {
                    let v = vals.get(&(k0 + 1).to_string())?;
                    let j = own
                        .and_then(|c| c.get(k0))
                        .and_then(|ch| find_choice(ch, v))
                        .or_else(|| find_choice(&it.choices, v))?;
                    same &= idx.get(k0).copied() == i64::try_from(j).ok();
                }
                Some(same)
            };
            match mapped(ans) {
                Some(same) => same,
                // Japanese and Chinese answers are sometimes separated by "、" instead of "；": try once more that way
                None if ans.contains('、') => mapped(&ans.replace('、', "；")) == Some(true),
                None => false,
            }
        }
    }
}

/// The six cells of a six-column row, in column order (A..F).
pub struct SixCells<'a> {
    pub en_question: &'a str,
    pub zh_question: &'a str,
    pub ja_question: &'a str,
    pub en_answer: &'a str,
    pub zh_answer: &'a str,
    pub ja_answer: &'a str,
}

/// Parses one row of the six-column layout. Returns the outcome (as for the older layouts) and the preview warnings of
/// the row (only for a readable row: the Chinese / Japanese answer differs from the English one).
///
/// The question is English-based end to end: `stem` / `options` / `items` / hotspot `choices` and the id come from
/// column A, `lang` is "en", `zh` comes from B and `ja` from C (each only when it lines up with the English), the answer
/// key comes from D. `explain` holds every language's explanation / memory aid / note, and `explanation` / `mnemonic`
/// / `note` keep one text for older clients (Chinese, else Japanese, else English).
pub fn parse_six_columns(
    sheet: &str,
    row: i64,
    cells: &SixCells<'_>,
) -> (QuestionOutcome, Vec<String>) {
    let text = norm_nl(cells.en_question);
    if text.is_empty() {
        return (
            QuestionOutcome::Skip(SKIP6_NO_ENGLISH.to_string()),
            Vec::new(),
        );
    }
    // Type detection on column A: a first line "HOTSPOT" (or ホットスポット / 热点题), or a first line that starts with
    // "HOTSPOT:" (the marker is then removed, as in the older layouts), marks a hotspot question; otherwise "A. …" option
    // lines make a choice question, and items ending in "Yes / No"-like choices a hotspot one
    let lead_hot = RES.hot_lead.is_match(&text);
    let text = if lead_hot {
        RES.hot_lead.replace(&text, NoExpand("")).into_owned()
    } else {
        text
    };
    let first_line = text
        .split('\n')
        .map(py_strip)
        .find(|ln| !ln.is_empty())
        .unwrap_or("");
    let marker = !lead_hot && is_hotspot_marker(first_line);
    let is_hot = lead_hot || marker;
    let body: &str = if marker {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };
    let (mut stem, mut options, mut items) = (String::new(), Vec::new(), Vec::new());
    if !is_hot {
        (stem, options) = parse_options(body);
    }
    if options.is_empty() {
        (stem, items) = parse_hotspot_en(body);
    }
    if options.is_empty() && items.is_empty() {
        return (
            QuestionOutcome::Skip(SKIP6_NO_STRUCTURE.to_string()),
            Vec::new(),
        );
    }

    // The other two languages of the question, aligned against the English structure
    let translation = py_strip(&question_text_of(&norm_nl(cells.zh_question))).to_string();
    let zh = if translation.is_empty() {
        None
    } else {
        parse_aligned(&translation, &options, &items)
    };
    let ja_question = question_text_of(&norm_nl(cells.ja_question));
    let ja = if py_strip(&ja_question).is_empty() {
        None
    } else {
        parse_aligned(&ja_question, &options, &items)
    };

    // The answer key: column D only
    let d = norm_nl(cells.en_answer);
    let d_secs = split_sections_with(&d, classify_heading_any);
    let Some(ans) = column_answer(&d, &d_secs) else {
        return (
            QuestionOutcome::Skip(SKIP6_NO_ANSWER.to_string()),
            Vec::new(),
        );
    };
    let other_langs: Vec<&Vec<Vec<String>>> = [zh.as_ref(), ja.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(|z| z.choices.as_ref())
        .collect();
    let (kind, answer) = match resolve_answer(&ans, &options, &items, &other_langs) {
        Ok(found) => found,
        Err(outcome) => return (outcome, Vec::new()),
    };

    // Explanations of every language, and the comparison of the E / F answers with D
    let en_text = ExplainText {
        explanation: d_secs.explanation.unwrap_or_default(),
        mnemonic: d_secs.mnemonic.unwrap_or_default(),
        note: answer_note(&ans),
    };
    let mut warnings = Vec::new();
    let mut other_text = |cell: &str, own: Option<&Zh>, warning: &str| -> ExplainText {
        let t = norm_nl(cell);
        let secs = split_sections_with(&t, classify_heading_any);
        let theirs = column_answer(&t, &secs);
        if let Some(a) = theirs.as_deref() {
            if !same_answer(a, &answer, &items, own.and_then(|z| z.choices.as_ref())) {
                warnings.push(warning.to_string());
            }
        }
        ExplainText {
            explanation: secs.explanation.unwrap_or_default(),
            mnemonic: secs.mnemonic.unwrap_or_default(),
            note: theirs.as_deref().map(answer_note).unwrap_or_default(),
        }
    };
    let zh_text = other_text(cells.zh_answer, zh.as_ref(), WARN6_ZH_DIFFERS);
    let ja_text = other_text(cells.ja_answer, ja.as_ref(), WARN6_JA_DIFFERS);

    // One text per field for older clients: Chinese, else Japanese, else English
    let pick = |field: fn(&ExplainText) -> &String| -> String {
        [&zh_text, &ja_text, &en_text]
            .into_iter()
            .map(field)
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_default()
    };
    let explanation = pick(|t| &t.explanation);
    let mnemonic = pick(|t| &t.mnemonic);
    let note = pick(|t| &t.note);
    let keep = |t: ExplainText| (!t.is_empty()).then_some(t);
    let explain = Explain {
        en: keep(en_text),
        zh: keep(zh_text),
        ja: keep(ja_text),
    };

    let first_text = first_text_of(&options, &items);
    let question = Question {
        kind,
        tag: if kind == Kind::Hotspot {
            "HOTSPOT".to_string()
        } else {
            String::new()
        },
        id: make_id(sheet, &stem, &first_text),
        stem,
        options,
        items,
        answer,
        translation,
        zh,
        // The base text is English already: no separate `english` / `en`
        english: String::new(),
        en: None,
        lang: "en".to_string(),
        ja,
        explanation,
        mnemonic,
        note,
        explain: Some(explain),
        sheet: sheet.to_string(),
        row,
    };
    (QuestionOutcome::Ok(Box::new(question)), warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(o: QuestionOutcome) -> Question {
        match o {
            QuestionOutcome::Ok(q) => *q,
            other => panic!("expected Ok, got {other:?}"),
        }
    }
    fn skip(o: QuestionOutcome) -> String {
        match o {
            QuestionOutcome::Skip(s) => s,
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn letters_forms() {
        let l = |s: &str| parse_letters(s).join("");
        assert_eq!(l("B"), "B");
        assert_eq!(l("A、C"), "AC");
        assert_eq!(l("AC"), "AC");
        assert_eq!(l("BA"), "AB");
        assert_eq!(l("b"), "B");
        assert_eq!(l("B（理由略）"), "B");
        assert_eq!(l("答えはB。"), "B");
        assert_eq!(l("Option B"), "B");
        assert_eq!(l(" A B C"), "ABC");
        assert_eq!(l("A（B）"), "A");
        assert_eq!(l("A、A、B"), "AB");
        assert_eq!(l("Ａ"), "");
        assert_eq!(l("A1"), "");
        assert_eq!(l("CAT"), "");
        assert_eq!(l("ABCDEFGH"), "");
        assert_eq!(l("ABCDEFGHI"), "");
        assert_eq!(l("(A)(B)"), "");
        assert_eq!(l("Z"), "");
        assert_eq!(l("\n\nC\nD"), "C");
    }

    #[test]
    fn item_answers() {
        let m = parse_item_answers("1 はい；2 いいえ").unwrap();
        assert_eq!(m.get("1").map(String::as_str), Some("はい"));
        assert_eq!(m.get("2").map(String::as_str), Some("いいえ"));
        // A single item with several words falls back to taking only up to the first whitespace (legacy defect, must be preserved)
        let m = parse_item_answers("1 Model Stage").unwrap();
        assert_eq!(m.get("1").map(String::as_str), Some("Model"));
        // Full-width digits
        let m = parse_item_answers("１ 是；２ 否").unwrap();
        assert_eq!(m.get("1").map(String::as_str), Some("是"));
        // Over-long number
        let long = format!("{} はい", "9".repeat(4301));
        assert!(parse_item_answers(&long).is_err());
    }

    #[test]
    fn canon_rules() {
        assert_eq!(canon(" Y. "), "\u{0}yes");
        assert_eq!(canon("いいえ"), "\u{0}no");
        assert_eq!(canon("Model-Stage"), canon("model stage"));
        assert_ne!(canon("Ｙｅｓ"), "\u{0}yes");
        assert_eq!(canon("ΑΣ"), "ας");
    }

    #[test]
    fn options_continuation_and_failures() {
        let (stem, o) = parse_options("題\nA. 甲\nB. 乙\nB. 乙二\nC. 丙");
        assert_eq!(stem, "題");
        assert_eq!(o.len(), 3);
        assert_eq!(o.get(1).map(|x| x.text.as_str()), Some("乙\nB. 乙二"));
        let (_, o) = parse_options("題\nA. 甲");
        assert!(o.is_empty());
        let (_, o) = parse_options("題\nB. 乙\nC. 丙");
        assert!(o.is_empty());
        // Letters only go up to H
        let body = "題\nA. a\nB. b\nC. c\nD. d\nE. e\nF. f\nG. g\nH. h\nI. i";
        let (_, o) = parse_options(body);
        assert_eq!(o.len(), 8);
        assert_eq!(o.last().map(|x| x.text.as_str()), Some("h\nI. i"));
        // Letter alone on its own line
        let (_, o) = parse_options("題\nA\n甲\nB\n乙");
        assert_eq!(o.len(), 2);
        assert_eq!(o.first().map(|x| x.text.as_str()), Some("甲"));
    }

    #[test]
    fn hotspot_basic_and_pending() {
        let body = "次の文。\n\n1. 太陽は東から昇る。\u{3000}はい / いいえ\n続きの説明行\n2. 月は恒星である。\u{3000}はい / いいえ";
        let (stem, items) = parse_hotspot(body);
        assert_eq!(stem, "次の文。");
        assert_eq!(items.len(), 2);
        assert_eq!(
            items.get(1).map(|i| i.text.as_str()),
            Some("続きの説明行\n月は恒星である。")
        );
        let (_, none) = parse_hotspot("はい/いいえ だけ");
        assert!(none.is_empty());
    }

    #[test]
    fn question_single_and_skips() {
        let q = ok(parse_question(
            "S",
            3,
            "題\nA. 甲\nB. 乙",
            "",
            "【答案】B\n\n【解析·怎么选】理由\n\n【记忆口诀】口",
        ));
        assert_eq!(q.kind, Kind::Single);
        assert_eq!(q.answer, Answer::Letters(vec!["B".to_string()]));
        assert_eq!(q.explanation, "理由");
        assert_eq!(q.mnemonic, "口");
        assert_eq!(q.row, 3);
        assert!(q.zh.is_none());
        assert_eq!(
            skip(parse_question("S", 1, "ただの文章です。", "", "【答案】A")),
            SKIP_NO_STRUCTURE
        );
        assert_eq!(
            skip(parse_question("S", 1, "題\nA. 甲\nB. 乙", "", "【解析】x")),
            SKIP_NO_ANSWER
        );
        assert_eq!(
            skip(parse_question("S", 1, "題\nA. 甲\nB. 乙", "", "【答案】E")),
            "答案 E 不在选项 A、B 里"
        );
        assert_eq!(
            skip(parse_question("S", 1, "題\nA. 甲\nB. 乙", "", "【答案】甲")),
            "答案「甲」里没有找到选项字母"
        );
    }

    #[test]
    fn question_hotspot_and_translation() {
        let a = "ホットスポット\n次の文。\n1. 一つ目。\u{3000}はい / いいえ\n2. 二つ目。\u{3000}はい / いいえ";
        let b = "【中文翻译】\n热点题：下面的句子。\n1. 第一项。\n2. 第二项。";
        let q = ok(parse_question("S", 1, a, b, "【答案】1 はい；2 いいえ"));
        assert_eq!(q.kind, Kind::Hotspot);
        assert_eq!(q.tag, "ホットスポット");
        assert_eq!(q.answer, Answer::Indexes(vec![0, 1]));
        let zh = q.zh.expect("zh");
        assert_eq!(zh.stem, "下面的句子。");
        assert_eq!(
            zh.items,
            Some(vec!["第一项。".to_string(), "第二项。".to_string()])
        );
        // Sub-item count does not match -> no zh
        let q = ok(parse_question(
            "S",
            1,
            a,
            "【中文翻译】\n1. 只有一项",
            "【答案】1 はい；2 いいえ",
        ));
        assert!(q.zh.is_none());
        assert!(!q.translation.is_empty());
        assert_eq!(
            skip(parse_question("S", 1, a, "", "【答案】1 はい")),
            "第 2 小项没有答案"
        );
        assert_eq!(
            skip(parse_question("S", 1, a, "", "【答案】1 maybe；2 いいえ")),
            "第 1 小项的答案「maybe」不在选项「はい / いいえ」里"
        );
        assert_eq!(
            skip(parse_question(
                "S",
                1,
                a,
                "",
                "【答案】1 はい；2 いいえ；3 はい"
            )),
            "答案里有第 3 项，但题目只有 2 个小项"
        );
    }
}
