//! 题目层：纯字符串函数，不依赖 XML。逐条对应 `app/xlsx_parser.py` 第 111-358 行。
//!
//! 约定：所有 `strip()` 用 Python 的空白集（`py_strip`）；所有长度按码点；正则里的 `\s` 一律写成
//! `[\s\x1c-\x1f]`（Python 的 `\s` 比 Rust 多 U+001C..U+001F），`\d` 一律用 Python 的 Nd 表。
use super::id::make_id;
use super::py_space::{
    char_len, digit_class_inner, digit_value, digits_to_canonical, py_strip, take_chars,
};
use crate::error::ImportError;
use crate::models::{Answer, En, Item, Kind, Opt, Question, Zh};
use regex::{NoExpand, Regex};
use std::collections::HashMap;
use std::sync::LazyLock;

/// 片段：Python `\s` 的字符类内部写法。
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

// ------------------------------------------------------------------ 标题 / 分段

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heading {
    Answer,
    Explanation,
    Mnemonic,
    Translation,
}

/// `classify_heading`：子串匹配，优先级 答案 > 解析 > 口诀 > 翻译。
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

/// `split_sections`：每类只取第一次出现；段文本到下一个已识别标题或末尾，再 strip。
pub fn split_sections(text: &str) -> Sections {
    let mut marks: Vec<(usize, usize, Heading)> = Vec::new();
    for m in RES.heading.captures_iter(text) {
        let (Some(whole), Some(name)) = (m.get(0), m.get(1)) else {
            continue;
        };
        if let Some(kind) = classify_heading(name.as_str()) {
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

// ------------------------------------------------------------------ 选项 / 热点

/// `OPT_LINE.match(ln)`：返回（大写字母，第 2 组文本；无则空串）。
fn opt_line(ln: &str) -> Option<(u8, String)> {
    let caps = RES.opt_line.captures(ln)?;
    let letter = caps.get(1)?.as_str().bytes().next()?.to_ascii_uppercase();
    let rest = caps.get(2).map_or("", |m| m.as_str()).to_string();
    Some((letter, rest))
}

/// 选择题：支持「A. 内容」和「A 换行 内容」。-> (题干, [选项])
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

/// `NUM_PREFIX.sub('', ln, count=1)`：返回去掉序号前缀后的文本。
fn strip_num_prefix(ln: &str) -> &str {
    match RES.num_prefix.find(ln) {
        Some(m) => ln.get(m.end()..).unwrap_or(ln),
        None => ln,
    }
}

/// 热点题：每个小项末尾带「はい / いいえ」之类的选项。-> (题干, [小项])
pub fn parse_hotspot(body: &str) -> (String, Vec<Item>) {
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
        if let Some(caps) = RES.hint.captures(text) {
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

// ------------------------------------------------------------------ 答案

/// `parse_letters`：括号内容整段替换成空格；只看第一个非空行；找两侧不是 ASCII 字母数字的 A-H/a-h。
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

/// 小项答案：序号（规范化十进制串）-> 答案文本。数字超过 4300 位（Python 在这里抛 ValueError）返回 Err。
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

/// `canon`：去首尾空白、小写、删分隔符；是 / 否的各种写法归一。
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

// ------------------------------------------------------------------ 翻译

/// `_norm_nl`
pub fn norm_nl(s: &str) -> String {
    py_strip(&s.replace("\r\n", "\n").replace('\r', "\n")).to_string()
}

/// 热点小项末尾的选项：「……。　是 / 否」。-> (去掉选项后的文字, 选项)。没有选项就原样返回。
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

/// 每个小项的本语言选项。只有**每个小项都写了、且个数和日语的一致**时才采用（一个个对上，不猜）；否则整题不提供，
/// 界面退回显示日语的选项。选项的位置就是答案下标，所以只要个数一致，「第 j 个」在三种语言里就是同一个意思。
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

/// 把 B 列的翻译拆成和日语题目一样的结构；对不上返回 None（不提供中文切换）。
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
        // 每个小项末尾如果写了本语言的选项（「……。　是 / 否」），把它拆出来；没写就还是原来的样子（choices = None）
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

// ------------------------------------------------------------------ 整题

pub const SKIP_NO_STRUCTURE: &str = "没有找到选项（A、B、C…）或带「はい / いいえ」的小项";
pub const SKIP_NO_ANSWER: &str = "没有找到答案（C 列要写「【答案】…」）";

/// `parse_question` 的结果：成功、读不出来（带原因）、或整个文件都要拒绝。
#[derive(Debug)]
pub enum QuestionOutcome {
    Ok(Box<Question>),
    Skip(String),
    Fatal(ImportError),
}

/// 旧的三列格式（A 日语、B 中文、C 答案）。现在只有单元测试直接用它；工作簿解析走 `parse_question_with_english`。
#[cfg(test)]
pub fn parse_question(sheet: &str, row: i64, a: &str, b: &str, c: &str) -> QuestionOutcome {
    parse_question_with_english(sheet, row, a, b, c, "")
}

/// 英语版拆成和日语一样的结构：单选 / 多选带 `options`，热点题带 `items` 和每个小项的英文选项。
/// 选项字母、小项个数和日语对不上就返回 None（页面不提供英语切换，而不是显示错位的内容）。
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

/// 四列格式里 A 列是英语：`english` 非空时额外拆出 `en`。其余逻辑和三列格式完全一样（题目 id 仍然来自日语）。
#[allow(clippy::needless_late_init)]
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

    // 中文 / 英语先拆好：热点题的答案可以用这两列里写的选项词来写（见下面）
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

    let kind;
    let answer;
    let first_text;
    if !options.is_empty() {
        let letters = parse_letters(&ans);
        let keys: Vec<&str> = options.iter().map(|o| o.key.as_str()).collect();
        if letters.is_empty() {
            return QuestionOutcome::Skip(format!(
                "答案「{}」里没有找到选项字母",
                take_chars(&ans, 20)
            ));
        }
        let bad: Vec<&str> = letters
            .iter()
            .map(String::as_str)
            .filter(|x| !keys.contains(x))
            .collect();
        if !bad.is_empty() {
            return QuestionOutcome::Skip(format!(
                "答案 {} 不在选项 {} 里",
                bad.join("、"),
                keys.join("、")
            ));
        }
        kind = if letters.len() > 1 {
            Kind::Multi
        } else {
            Kind::Single
        };
        answer = Answer::Letters(letters);
        first_text = options.first().map(|o| o.text.clone()).unwrap_or_default();
    } else {
        let Ok(vals) = parse_item_answers(&ans) else {
            return QuestionOutcome::Fatal(ImportError::Message(
                "答案里有超长的数字（超过 4300 位），已拒绝。".to_string(),
            ));
        };
        let mut idx: Vec<i64> = Vec::new();
        // 中文 / 英语两列里各小项末尾写的选项（个数和日语一致才有；对不上的那一列不参与匹配）
        let other_langs: Vec<&Vec<Vec<String>>> = [zh.as_ref(), en.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|z| z.choices.as_ref())
            .collect();
        for (k0, it) in items.iter().enumerate() {
            let k = k0 + 1;
            let Some(v) = vals.get(&k.to_string()) else {
                return QuestionOutcome::Skip(format!("第 {k} 小项没有答案"));
            };
            // 答案可以用日语、中文或英语任意一种语言的选项词来写：三列的第 j 个选项是一一对应的（同一个下标），
            // 所以先在日语选项里找，找不到再依次在中文、英语那一列写的选项里找。
            let Some(j) = find_choice(&it.choices, v).or_else(|| {
                other_langs
                    .iter()
                    .find_map(|c| c.get(k0).and_then(|ch| find_choice(ch, v)))
            }) else {
                return QuestionOutcome::Skip(format!(
                    "第 {k} 小项的答案「{v}」不在选项「{}」里",
                    it.choices.join(" / ")
                ));
            };
            idx.push(i64::try_from(j).unwrap_or(0));
        }
        let mut extra: Vec<&String> = vals
            .keys()
            // 键在 1..=N 里，当且仅当它恰好等于某个 k 的十进制写法（不带前导零或加号）；O(1) 判断，不再为每个键扫一遍 N 项
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
            return QuestionOutcome::Skip(format!(
                "答案里有第 {joined} 项，但题目只有 {} 个小项",
                items.len()
            ));
        }
        kind = Kind::Hotspot;
        answer = Answer::Indexes(idx);
        first_text = items.first().map(|i| i.text.clone()).unwrap_or_default();
    }

    let note = RES
        .note
        .captures_iter(&ans)
        .filter_map(|caps| caps.get(1))
        .map(|m| py_strip(m.as_str()))
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("；");
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
        explanation: secs.explanation.unwrap_or_default(),
        mnemonic: secs.mnemonic.unwrap_or_default(),
        note,
        sheet: sheet.to_string(),
        row,
    }))
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
        // 单项多词回退只取到空白为止（旧版缺陷，需保持）
        let m = parse_item_answers("1 Model Stage").unwrap();
        assert_eq!(m.get("1").map(String::as_str), Some("Model"));
        // 全角数字
        let m = parse_item_answers("１ 是；２ 否").unwrap();
        assert_eq!(m.get("1").map(String::as_str), Some("是"));
        // 超长数字
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
        // 字母只到 H
        let body = "題\nA. a\nB. b\nC. c\nD. d\nE. e\nF. f\nG. g\nH. h\nI. i";
        let (_, o) = parse_options(body);
        assert_eq!(o.len(), 8);
        assert_eq!(o.last().map(|x| x.text.as_str()), Some("h\nI. i"));
        // 字母单独成行
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
        // 小项数对不上 -> 没有 zh
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
