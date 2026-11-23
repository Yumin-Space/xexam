//! Cleaning and checking the answer of the AI study assessment (v2.7). Small models sometimes count out loud, show
//! drafts or get cut off; the assessment is only shown (and stored) when the cleaned text passes [`check_assess`].
//! Everything here is pure.
use crate::prompt::Lang;

/// Why a cleaned answer was refused. Used in logs only (never the text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssessReject {
    Empty,
    TooShort,
    TooLong,
    /// Does not end with a closing sentence mark: probably cut off.
    Unfinished,
    /// Per-character numbering such as `字(12)` or `3:word`.
    Counting,
    /// Drafts, checks, notes or leaked prompt text.
    ProcessText,
    /// A list instead of a paragraph.
    List,
    WrongLanguage,
}

/// The tag names of reasoning spans some gateways leave inside the content.
const THINK_TAGS: [&str; 3] = ["think", "thinking", "thought"];

fn is_wide(c: char) -> bool {
    (c as u32) >= 0x2E00
}

/// Removes `<think>…</think>` style spans (an unterminated one up to the end) and a stray closing tag with whatever
/// precedes it (the opening tag was split off upstream).
fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    loop {
        let low = out.to_ascii_lowercase();
        let mut best: Option<(usize, &str)> = None;
        for t in THINK_TAGS {
            if let Some(i) = low.find(&format!("<{t}>")) {
                if best.is_none_or(|(b, _)| i < b) {
                    best = Some((i, t));
                }
            }
        }
        let Some((i, t)) = best else { break };
        let open_len = t.len() + 2;
        let close = format!("</{t}>");
        match low[i + open_len..].find(&close) {
            Some(j) => out.replace_range(i..i + open_len + j + close.len(), ""),
            None => out.truncate(i),
        }
    }
    let low = out.to_ascii_lowercase();
    let mut cut = 0;
    for t in THINK_TAGS {
        let close = format!("</{t}>");
        if let Some(i) = low.rfind(&close) {
            cut = cut.max(i + close.len());
        }
    }
    out.split_off(cut)
}

/// A list marker at the start of a (trimmed) line: `- `, `* `, a bullet, `1. ` or `1) `. Returns the rest.
fn list_item(line: &str) -> Option<&str> {
    for p in ["- ", "* ", "• ", "– ", "· "] {
        if let Some(r) = line.strip_prefix(p) {
            return Some(r);
        }
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=3).contains(&digits) {
        let rest = &line[digits..];
        for p in [". ", ") "] {
            if let Some(r) = rest.strip_prefix(p) {
                return Some(r);
            }
        }
    }
    None
}

fn strip_heading_and_quote(mut line: &str) -> &str {
    loop {
        let t = line.trim_start();
        if let Some(r) = t.strip_prefix('#') {
            line = r.trim_start_matches('#');
        } else if let Some(r) = t.strip_prefix("> ") {
            line = r;
        } else {
            return t;
        }
    }
}

fn unwrap_quotes(s: &str) -> &str {
    for (o, c) in [
        ('"', '"'),
        ('“', '”'),
        ('「', '」'),
        ('『', '』'),
        ('\'', '\''),
        ('‘', '’'),
    ] {
        if let Some(inner) = s.strip_prefix(o).and_then(|r| r.strip_suffix(c)) {
            if !inner.contains(o) {
                return inner.trim();
            }
        }
    }
    s
}

/// Cleans a raw answer into one paragraph: reasoning spans, code fences, Markdown markers and wrapping quotes are
/// removed and the lines are joined (a single space between Latin lines, none next to CJK text). When two or more lines
/// are list items the answer is not a paragraph: the markers are kept (and the lines stay separate) so that
/// [`check_assess`] refuses it as [`AssessReject::List`].
pub fn clean_assess(raw: &str) -> String {
    let text = strip_think(raw).replace("**", "").replace("__", "");
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("```") && !l.is_empty())
        .map(strip_heading_and_quote)
        .filter(|l| !l.is_empty())
        .collect();
    let items = lines.iter().filter(|l| list_item(l).is_some()).count();
    if items >= 2 {
        return lines.join("\n");
    }
    let mut out = String::new();
    for l in lines {
        let l = list_item(l).unwrap_or(l).trim();
        if l.is_empty() {
            continue;
        }
        if let (Some(a), Some(b)) = (out.chars().last(), l.chars().next()) {
            if !is_wide(a) && !is_wide(b) {
                out.push(' ');
            }
        }
        out.push_str(l);
    }
    unwrap_quotes(out.trim()).to_string()
}

fn is_cjk(c: char) -> bool {
    let u = c as u32;
    (0x3400..=0x4DBF).contains(&u)
        || (0x4E00..=0x9FFF).contains(&u)
        || (0x3040..=0x30FF).contains(&u)
}

/// `(12)` or `（12）` after a character (optionally after a space): how a model numbers characters or words while counting.
fn bracket_numbers(s: &str) -> usize {
    let cs: Vec<char> = s.chars().collect();
    let mut n = 0;
    let mut i = 1;
    while i < cs.len() {
        // whitespace before the bracket is fine: ") 匀 (102) 摊 (103)" is counting too
        if cs[i] == '(' || cs[i] == '（' {
            let mut j = i + 1;
            while j < cs.len() && cs[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j - i - 1 <= 4 && j < cs.len() && (cs[j] == ')' || cs[j] == '）') {
                n += 1;
                i = j;
            }
        }
        i += 1;
    }
    n
}

/// `12:word` / `12. word` tokens (a number of at most three digits at a token start, then a word).
fn numbered_tokens(s: &str) -> usize {
    let cs: Vec<char> = s.chars().collect();
    let mut n = 0;
    let mut i = 0;
    while i < cs.len() {
        if cs[i].is_ascii_digit() && (i == 0 || cs[i - 1].is_whitespace()) {
            let mut j = i;
            while j < cs.len() && cs[j].is_ascii_digit() {
                j += 1;
            }
            if j - i <= 3 && j + 1 < cs.len() {
                let word = |c: char| c.is_alphabetic();
                let hit = match cs[j] {
                    ':' => word(cs[j + 1]),
                    '.' => cs[j + 1] == ' ' && j + 2 < cs.len() && word(cs[j + 2]),
                    _ => false,
                };
                if hit {
                    n += 1;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    n
}

/// Three or more bare small numbers in a row (`1 2 3`, `0,1,2,3`).
fn number_run(s: &str) -> bool {
    let mut run = 0;
    for tok in s.split_whitespace() {
        let parts: Vec<&str> = tok
            .trim_end_matches([',', '，'])
            .split([',', '，'])
            .collect();
        let bare = parts
            .iter()
            .all(|p| (1..=3).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()));
        if bare {
            run += parts.len();
            if run >= 3 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

fn has_snake_token(s: &str) -> bool {
    s.split(|c: char| !(c.is_ascii_lowercase() || c == '_'))
        .any(|t| {
            t.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
                && t.find('_').is_some_and(|i| i + 1 < t.len())
        })
}

/// Phrases that only appear when the model leaks its working: counting aloud, constraint checks, drafts, the prompt's
/// own tags. Everyday coaching words ("let's aim to", "verify each answer", "a first draft of a habit", "time
/// constraints") are NOT in this list: a plain word test refused good English replies.
const PROCESS_PHRASES: [&str; 19] = [
    "total length",
    "word count",
    "character count",
    "char count",
    "let me count",
    "let's count",
    "lets count",
    "let me check",
    "let me verify",
    "let me draft",
    "verify constraints",
    "constraints:",
    "constraint check",
    "draft:",
    "(draft",
    "part 1",
    "part one",
    "<facts>",
    "</facts>",
];

/// `wait,` (a model correcting itself) only where a sentence or a bracket starts; "please wait, then" is ordinary.
fn has_wait_correction(low: &str) -> bool {
    let b = low.as_bytes();
    let mut from = 0;
    while let Some(i) = low[from..].find("wait,") {
        let at = from + i;
        let before = low[..at].trim_end();
        let start = before.is_empty()
            || before.ends_with(['(', '（', '.', '!', '?', '。', '！', '？', '\n', ':', '*']);
        if start {
            return true;
        }
        from = at + 5;
        if from >= b.len() {
            break;
        }
    }
    false
}

fn has_process_text(s: &str) -> bool {
    let low = s.to_lowercase();
    PROCESS_PHRASES.iter().any(|w| low.contains(w))
        || low.contains("<stats>")
        || has_wait_correction(&low)
        || has_snake_token(s)
}

/// Checks a text that went through [`clean_assess`]. `Ok` means it reads like the wanted short paragraph.
pub fn check_assess(text: &str, lang: Lang) -> Result<(), AssessReject> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AssessReject::Empty);
    }
    if text
        .lines()
        .filter(|l| list_item(l.trim()).is_some())
        .count()
        >= 2
    {
        return Err(AssessReject::List);
    }
    if has_process_text(text) {
        return Err(AssessReject::ProcessText);
    }
    if bracket_numbers(text) >= 3 || numbered_tokens(text) >= 3 || number_run(text) {
        return Err(AssessReject::Counting);
    }
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    let cjk = text.chars().filter(|c| is_cjk(*c)).count();
    let share = if letters == 0 {
        0.0
    } else {
        cjk as f64 / letters as f64
    };
    let wrong = match lang {
        Lang::Zh | Lang::Ja => share < 0.6,
        Lang::En => share > 0.2,
    };
    if wrong {
        return Err(AssessReject::WrongLanguage);
    }
    let (min, max, len) = match lang {
        Lang::Zh | Lang::Ja => (30, 260, text.chars().filter(|c| !c.is_whitespace()).count()),
        Lang::En => (15, 170, text.split_whitespace().count()),
    };
    if len < min {
        return Err(AssessReject::TooShort);
    }
    if len > max {
        return Err(AssessReject::TooLong);
    }
    let end = text.chars().last().unwrap_or(' ');
    if !"。！？.!?…」』”\"）)".contains(end) {
        return Err(AssessReject::Unfinished);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_ZH: &str = "细小的溪流日夜不停，终会把石头磨出一条路。你最近保持了稳定的练习，正确率也不错。接下来可以试着把错题本里的题每天清理几道。愿你慢慢走，也能走得很远。";
    const GOOD_EN: &str = "Rain falls one drop at a time, yet it fills the deepest rivers. You have been showing up on a steady rhythm, and most of your answers are landing well. Try clearing a few wrong-answer notebook questions at the start of each session. Take it one calm step at a time.";
    const GOOD_JA: &str = "小さな雫が集まって、やがて大きな川になります。最近は落ち着いたペースで練習が続いていて、とても良い流れです。次は、間違えた問題を毎回数問だけ見直してみましょう。ゆっくり進んでいきましょう。";

    fn run(raw: &str, lang: Lang) -> Result<(), AssessReject> {
        check_assess(&clean_assess(raw), lang)
    }

    #[test]
    fn good_replies_pass() {
        assert_eq!(run(GOOD_ZH, Lang::Zh), Ok(()));
        assert_eq!(run(GOOD_EN, Lang::En), Ok(()));
        assert_eq!(run(GOOD_JA, Lang::Ja), Ok(()));
        // with wrappers a model likes to add
        assert_eq!(run(&format!("**{GOOD_ZH}**"), Lang::Zh), Ok(()));
        assert_eq!(run(&format!("\"{GOOD_EN}\""), Lang::En), Ok(()));
        assert_eq!(
            run(
                &format!("<think>count the words first</think>\n{GOOD_EN}"),
                Lang::En
            ),
            Ok(())
        );
        assert_eq!(run(&format!("「{GOOD_JA}」"), Lang::Ja), Ok(()));
    }

    #[test]
    fn numbers_and_percents_in_good_text_are_fine() {
        let t = "春雨一点一点，终会润湿整片田野。你最近完成了340道题，正确率约74%，节奏很稳。明天先花十分钟回看错题本里的几道题。慢慢来就好。";
        assert_eq!(run(t, Lang::Zh), Ok(()));
        let e = "A river is only patient water that never stopped moving. You answered 340 questions at about 74% right, which is a calm, steady pace. Tomorrow, spend ten minutes on a few notebook questions. Small steps are enough.";
        assert_eq!(run(e, Lang::En), Ok(()));
    }

    #[test]
    fn per_character_numbering_is_counting() {
        let zh = "山(1)间(2)的(3)清(4)泉(5)静(6)静(7)流(8)淌(9)着(10)，你(11)今(12)天(13)很(14)认(15)真(16)地(17)学(18)习(19)了(20)。";
        assert_eq!(run(zh, Lang::Zh), Err(AssessReject::Counting));
        let en = "not 10:through 11:sudden 12:force 13:small 14:patient 15:steps 16:add 17:up to something quite vast and lasting.";
        assert_eq!(run(en, Lang::En), Err(AssessReject::Counting));
        assert_eq!(
            run(
                "1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 done today.",
                Lang::En
            ),
            Err(AssessReject::Counting)
        );
    }

    #[test]
    fn process_text_is_refused() {
        let total = "细小的溪流日夜不停，终会把石头磨出一条路。你最近保持了稳定的练习。接下来每天清理几道错题。Total length: 144 characters。";
        assert_eq!(run(total, Lang::Zh), Err(AssessReject::ProcessText));
        let verify = "7. Verify Constraints:\n* One short paragraph? Yes.\n* Two parts? Yes, these are all fine and well and good for the reader to see.";
        assert!(run(verify, Lang::En).is_err());
        let wait = "细小的溪流日夜不停，终会把石头磨出一条路。你最近保持了稳定的练习。（Wait, typo check）接下来每天清理几道错题。愿你慢慢走。";
        assert_eq!(run(wait, Lang::Zh), Err(AssessReject::ProcessText));
        let key = "Rain fills rivers one drop at a time, and your wrong_notebook_open number is the next small thing to work on, gently and with patience, day by day.";
        assert_eq!(run(key, Lang::En), Err(AssessReject::ProcessText));
        let facts = "<facts> Rain fills rivers one drop at a time, and you keep showing up for it, which is a good thing to notice about yourself today.";
        assert_eq!(run(facts, Lang::En), Err(AssessReject::ProcessText));
        let lets = "Wait, let's just count all characters including punctuation as standard and try again now to be sure that it is good enough for the user.";
        assert_eq!(run(lets, Lang::En), Err(AssessReject::ProcessText));
    }

    #[test]
    fn everyday_coaching_words_are_not_process_text() {
        for t in [
            "Rain fills rivers drop by drop, and your answers have been landing well lately. Let's aim to clear a few notebook questions each morning. Small steps are enough.",
            "Water wears stone by staying patient. You are doing steadily and most answers are right. Next, verify each answer you flagged before you move on. Keep it calm.",
            "A seed grows quietly, one day at a time. Even with time constraints you studied on several days. Treat each session as a good first draft of a habit, and keep going.",
            "Rain fills rivers drop by drop. Let's check the open notebook questions tomorrow. You are doing well and the pace is calm.",
            "Water wears stone by staying patient. Let's verify the answers you flagged before moving on. Keep it calm and steady.",
            "A seed grows quietly over many days. Take step 1 today: ten minutes on the notebook. Then rest, you have earned it.",
            "Small ripples become tides over time. Let me suggest ten minutes on the notebook tomorrow, since that is where the next gain is. You are on a good path.",
        ] {
            assert_eq!(run(t, Lang::En), Ok(()), "{t}");
        }
    }

    #[test]
    fn spaced_bracket_counting_is_counting() {
        let t = "你(100)今(101)天(102)很 好。匀 (102) 摊 (103) 开 (104) 来 (105) 慢慢走。愿你一直走下去，也一直这样认真地走。";
        assert_eq!(run(t, Lang::Zh), Err(AssessReject::Counting));
        let spaced = ") 匀 (102) 摊 (103) 推 (104) 敲 (105) 夜。细小的溪流日夜不停，终会把石头磨出一条路。你最近保持了稳定的练习，慢慢来。";
        assert_eq!(run(spaced, Lang::Zh), Err(AssessReject::Counting));
    }

    #[test]
    fn the_owners_captured_junk_stays_refused() {
        for t in [
            "Rain falls one drop at a time and it fills rivers. Wait, let's just count all characters including punctuation as standard and try again now to be sure it is good.",
            "Rain falls one drop at a time and it fills rivers, which is a calm and steady picture to hold on to. Total length: 144 characters. You keep showing up each day.",
            "(Draft 1) Rain falls one drop at a time and it fills rivers, which is a calm picture to hold on to. You keep showing up for it, and that matters a great deal.",
            "Rain falls one drop at a time and it fills rivers. Verify Constraints: one short paragraph, a nature image, a next step. You keep showing up for it, which is good.",
            "Rain falls one drop at a time and it fills rivers, and you keep showing up. (Wait, typo check) Try the notebook tomorrow for ten minutes, as a small calm step.",
        ] {
            assert_eq!(run(t, Lang::En), Err(AssessReject::ProcessText), "{t}");
        }
    }

    #[test]
    fn cut_and_short_replies_are_refused() {
        let cut = "细小的溪流日夜不停，终会把石头磨出一条路。你最近保持了稳定的练习，正确率也不错。接下来可以试着把错题本里的题每天清理几道，让夜";
        assert_eq!(run(cut, Lang::Zh), Err(AssessReject::Unfinished));
        assert_eq!(run("夜(12", Lang::Zh), Err(AssessReject::TooShort));
        assert_eq!(
            run("Those who study hard often find success.", Lang::En),
            Err(AssessReject::TooShort)
        );
        assert_eq!(run("", Lang::En), Err(AssessReject::Empty));
        assert_eq!(
            run("<think>only thoughts, never closed", Lang::En),
            Err(AssessReject::Empty)
        );
        let long = GOOD_ZH.repeat(8);
        assert_eq!(run(&long, Lang::Zh), Err(AssessReject::TooLong));
        let en_cut = "Rain falls one drop at a time, yet it fills the deepest rivers. You have been showing up on a steady rhythm, and most of your answers are landing";
        assert_eq!(run(en_cut, Lang::En), Err(AssessReject::Unfinished));
    }

    #[test]
    fn lists_are_refused() {
        let l = "- 每天练习二十分钟，保持稳定的节奏。\n- 复习错题本里的题目，每次几道就好。\n- 睡前回顾一下今天的收获吧。";
        assert_eq!(run(l, Lang::Zh), Err(AssessReject::List));
        let n = "1. Study a little each day, calmly and steadily for a while.\n2. Clear a few notebook questions every session, as you can.";
        assert_eq!(run(n, Lang::En), Err(AssessReject::List));
        // a single marker is just stripped
        let one = format!("- {GOOD_EN}");
        assert_eq!(run(&one, Lang::En), Ok(()));
    }

    #[test]
    fn wrong_language_is_refused() {
        assert_eq!(run(GOOD_EN, Lang::Zh), Err(AssessReject::WrongLanguage));
        assert_eq!(run(GOOD_ZH, Lang::En), Err(AssessReject::WrongLanguage));
    }

    #[test]
    fn clean_removes_wrappers_and_joins_lines() {
        assert_eq!(
            clean_assess("<thinking>x y</thinking>Hello there."),
            "Hello there."
        );
        assert_eq!(clean_assess("<THINK>x</THINK>Hi"), "Hi");
        assert_eq!(
            clean_assess("draft words</think>Final text."),
            "Final text."
        );
        assert_eq!(clean_assess("Keep <thought>never closed"), "Keep");
        assert_eq!(clean_assess("```text\nOne line.\n```"), "One line.");
        assert_eq!(
            clean_assess("# Title\n**Bold** words\nmore words"),
            "Title Bold words more words"
        );
        assert_eq!(clean_assess("第一句。\n第二句。"), "第一句。第二句。");
        assert_eq!(clean_assess("“引用的文字。”"), "引用的文字。");
        assert_eq!(clean_assess("1. Only one."), "Only one.");
        assert_eq!(clean_assess("a\n\n  b"), "a b");
        // two list lines stay separate so the check can refuse them
        assert_eq!(clean_assess("- a\n- b"), "- a\n- b");
    }
}
