//! The system prompt and the history sent with a message (spec 4.5; reply language and free mode: round 2, 5.2 / 7).
use crate::limits::MEMORY_PROMPT_CHARS;
use crate::provider::{Role, Turn};
use crate::util::char_len;

/// The reply language: the language chosen in the user's Xexam settings, sent by the frontend as `lang`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    Zh,
    Ja,
}

impl Lang {
    /// `"en"`, `"zh"` (also the alias `"zh-CN"`) or `"ja"`; anything else is not a language of the app.
    pub fn parse(s: &str) -> Option<Lang> {
        match s {
            "en" => Some(Lang::En),
            "zh" | "zh-CN" => Some(Lang::Zh),
            "ja" => Some(Lang::Ja),
            _ => None,
        }
    }

    /// The language's English name, as written into the prompts.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Zh => "Simplified Chinese",
            Lang::Ja => "Japanese",
        }
    }

    /// The name of the language of a question text, as written into the `language` attribute of a `<question>` tag.
    pub fn question_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Zh => "Chinese",
            Lang::Ja => "Japanese",
        }
    }
}

/// The reply-language rule; `{language}` is [`Lang::name`]. It is the whole system prompt of a free-mode chat.
pub const LANGUAGE_LINE: &str = "Always reply in {language} (the language chosen in the user's Xexam settings), unless the user explicitly asks for another language in this message.";

/// The static part of the system prompt (the language line and the mode line follow the memories, at the end, so that
/// the start of the prompt stays identical between requests); `{date}` is today's local date.
pub const SYSTEM_PROMPT: &str = "You are the study assistant inside Xexam, a self-hosted exam practice app. Answer clearly and concisely.
If you are not sure, say so. Use plain text; simple Markdown
(bold, inline code, code blocks, bullet lists) is fine. If the user asks you to remember something, answer
briefly; the app offers to save it to the user's memory. Today is {date}.";

/// Added after the language line at the end of the normal-mode system prompt only when the history sent contains free-mode messages.
pub const NORMAL_MODE_BLOCK: &str =
    "Current mode: normal (Xexam study assistant); earlier turns may be free mode.";

/// Added after the language line of the free-mode system prompt only when the history sent contains normal-mode
/// messages. It only lifts Xexam's own role, nothing of the provider's rules.
pub const FREE_MODE_BLOCK: &str = "Current mode: free (general assistant, not the study-assistant role); earlier turns may be normal mode.";

/// Introduces the user's memories (appended after a blank line).
pub const MEMORY_INTRO: &str =
    "The user asked you to remember these notes (they may be out of date; never mention this list unless asked):";

/// System prompt of the test call (no language line: the answer is checked for any text).
pub const VERIFY_PROMPT: &str = "Reply with the single word OK.";
/// User message of the test call.
pub const VERIFY_MESSAGE: &str = "Test";

/// System prompt of the memory suggestion; `{language}` is [`Lang::name`].
pub const SUGGEST_PROMPT: &str = "Rewrite the user's request as one short note about the user to remember for later chats. Write the note in {language}. At most 200 characters. Reply with the note only.";
/// Characters of the message sent for a memory suggestion.
pub const SUGGEST_INPUT_CHARS: usize = 2000;

/// The reply-language rule for `lang`.
pub fn language_line(lang: Lang) -> String {
    LANGUAGE_LINE.replace("{language}", lang.name())
}

/// The system prompt with the language, the date and the memories (newest-updated first, whole notes only, at most
/// 4 000 characters).
pub fn system_prompt(lang: Lang, date: &str, memories: &[String], switched: bool) -> String {
    let mut s = SYSTEM_PROMPT.replace("{date}", date);
    let mut used = 0usize;
    let mut lines = Vec::new();
    for m in memories {
        let n = char_len(m);
        if used + n > MEMORY_PROMPT_CHARS {
            break;
        }
        used += n;
        lines.push(format!("- {m}"));
    }
    if !lines.is_empty() {
        s.push_str("\n\n");
        s.push_str(MEMORY_INTRO);
        for l in lines {
            s.push('\n');
            s.push_str(&l);
        }
    }
    // The per-request part comes last: the language line, then the mode line after a switch
    s.push_str("\n\n");
    s.push_str(&language_line(lang));
    if switched {
        s.push('\n');
        s.push_str(NORMAL_MODE_BLOCK);
    }
    s
}

/// The system prompt of a free-mode chat (administrators only): the language line alone, followed by the free-mode
/// line when `switched`; no study-assistant instructions, date or memories.
pub fn free_prompt(lang: Lang, switched: bool) -> String {
    if switched {
        format!("{}\n{FREE_MODE_BLOCK}", language_line(lang))
    } else {
        language_line(lang)
    }
}

/// The system prompt of a memory suggestion in `lang`.
pub fn suggest_prompt(lang: Lang) -> String {
    SUGGEST_PROMPT.replace("{language}", lang.name())
}

/// A previous message of the chat.
pub struct Prev<'a> {
    pub role: Role,
    pub content: &'a str,
    pub status: &'a str,
    /// Written in free mode
    pub free: bool,
}

/// The history sent upstream and whether it holds a message written in the other mode than the current one.
pub struct History {
    pub turns: Vec<Turn>,
    pub other_mode: bool,
}

/// Messages of the history that are sent in full (the newest ones).
pub const FULL_RECENT: usize = 4;
/// Older assistant messages keep this many characters, then [`ASSISTANT_CUT_MARK`].
pub const OLD_ASSISTANT_CHARS: usize = 800;
pub const ASSISTANT_CUT_MARK: &str = "…(shortened)";
/// Older user messages keep this many characters, then [`USER_CUT_MARK`].
pub const OLD_USER_CHARS: usize = 2000;
pub const USER_CUT_MARK: &str = "…";

/// The shortening line and the window start move in steps of this many messages (4 turns), so the request between two
/// moves is the previous request plus the new messages: the provider's prompt cache keeps hitting.
pub const STEP: usize = 8;

/// The text of a history message as sent: the newest [`FULL_RECENT`] messages (`rank` 0 = newest) in full, older ones
/// cut to a fixed prefix. The cut depends only on the message and on being old, so the same message always gives the
/// same text. (`history` decides what counts as old with the stepped line `B`, not with the rank, and passes
/// [`FULL_RECENT`] for a message below the line.)
pub fn shortened(role: Role, content: &str, rank: usize) -> String {
    if rank < FULL_RECENT {
        return content.to_string();
    }
    let (keep, mark) = match role {
        Role::Assistant => (OLD_ASSISTANT_CHARS, ASSISTANT_CUT_MARK),
        _ => (OLD_USER_CHARS, USER_CUT_MARK),
    };
    match content.char_indices().nth(keep) {
        Some((i, _)) => format!("{}{mark}", &content[..i]),
        None => content.to_string(),
    }
}

/// The shortening line for `n` usable earlier messages: `floor(max(0, n - FULL_RECENT) / STEP) * STEP`. Messages with an
/// index below it (oldest first) are sent shortened, the others in full.
pub fn shortening_line(n: usize) -> usize {
    n.saturating_sub(FULL_RECENT) / STEP * STEP
}

/// The messages sent upstream (A2, stateless): the earlier `done` / `stopped` messages with text, oldest first, those
/// below the shortening line [`shortening_line`] cut ([`shortened`]), starting at the smallest multiple of [`STEP`] at
/// which the history and the new message fit `budget` characters (shortened lengths counted); when no multiple fits,
/// newest first while it fits (never half a message). A leading assistant message is dropped. `prev` is the path of the
/// viewed branch, oldest first.
pub fn history(prev: &[Prev<'_>], new_message: &str, budget: usize, free: bool) -> History {
    let usable: Vec<&Prev<'_>> = prev
        .iter()
        .filter(|p| (p.status == "done" || p.status == "stopped") && !p.content.is_empty())
        .collect();
    let n = usable.len();
    let line = shortening_line(n);
    let shown: Vec<String> = usable
        .iter()
        .enumerate()
        .map(|(i, p)| shortened(p.role, p.content, if i < line { FULL_RECENT } else { 0 }))
        .collect();
    let lens: Vec<usize> = shown.iter().map(|t| char_len(t)).collect();
    let new_len = char_len(new_message);
    // suffix[i] = characters of messages i..n
    let mut suffix = vec![0usize; n + 1];
    for i in (0..n).rev() {
        suffix[i] = suffix[i + 1] + lens.get(i).copied().unwrap_or(0);
    }
    let start = (0..n)
        .step_by(STEP)
        .find(|s| suffix.get(*s).is_some_and(|c| c + new_len <= budget))
        .unwrap_or_else(|| {
            // today's rule: newest first while the total fits
            let mut total = new_len;
            let mut first = n;
            for i in (0..n).rev() {
                let l = lens.get(i).copied().unwrap_or(0);
                if total + l > budget {
                    break;
                }
                total += l;
                first = i;
            }
            first
        });
    let mut picked: Vec<(Turn, bool)> = usable
        .iter()
        .zip(shown)
        .skip(start)
        .map(|(p, content)| {
            (
                Turn {
                    role: p.role,
                    content,
                },
                p.free,
            )
        })
        .collect();
    while picked.first().is_some_and(|t| t.0.role == Role::Assistant) {
        picked.remove(0);
    }
    let other_mode = picked.iter().any(|t| t.1 != free);
    let mut turns: Vec<Turn> = picked.into_iter().map(|t| t.0).collect();
    turns.push(Turn {
        role: Role::User,
        content: new_message.to_string(),
    });
    History { turns, other_mode }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The v0.8 first-build prompt: round 2 replaces exactly its language sentence.
    const FIRST_BUILD_PROMPT: &str = "You are the study assistant inside Xexam, a self-hosted exam practice app. Answer clearly and concisely.
Reply in the language the user writes in. If you are not sure, say so. Use plain text; simple Markdown
(bold, inline code, code blocks, bullet lists) is fine. If the user asks you to remember something, answer
briefly; the app offers to save it to the user's memory. Today is {date}.";

    #[test]
    fn languages() {
        assert_eq!(Lang::parse("en"), Some(Lang::En));
        assert_eq!(Lang::parse("zh"), Some(Lang::Zh));
        assert_eq!(Lang::parse("zh-CN"), Some(Lang::Zh));
        assert_eq!(Lang::parse("ja"), Some(Lang::Ja));
        for bad in ["", "fr", "EN", "zh-TW", "zh-cn", " en", "ja-JP"] {
            assert_eq!(Lang::parse(bad), None, "{bad:?}");
        }
        assert_eq!(
            language_line(Lang::Ja),
            "Always reply in Japanese (the language chosen in the user's Xexam settings), unless the user explicitly asks for another language in this message."
        );
        assert!(language_line(Lang::Zh).starts_with("Always reply in Simplified Chinese ("));
        assert!(language_line(Lang::En).starts_with("Always reply in English ("));
    }

    #[test]
    fn only_the_language_sentence_changed() {
        // the language sentence moved to the end of the prompt: the rest of the text is the first build's
        assert_eq!(
            SYSTEM_PROMPT,
            FIRST_BUILD_PROMPT.replace("Reply in the language the user writes in. ", "")
        );
        let s = system_prompt(Lang::Zh, "2026-01-02", &[], false);
        assert_eq!(
            s,
            format!(
                "{}\n\n{}",
                FIRST_BUILD_PROMPT
                    .replace("Reply in the language the user writes in. ", "")
                    .replace("{date}", "2026-01-02"),
                language_line(Lang::Zh)
            )
        );
        let m = system_prompt(Lang::Zh, "2026-01-02", &[], true);
        assert_eq!(m, format!("{s}\n{NORMAL_MODE_BLOCK}"));
        assert!(!s.contains('{') && !m.contains('{'));
    }

    #[test]
    fn free_and_suggest_prompts() {
        assert_eq!(free_prompt(Lang::En, false), language_line(Lang::En));
        let f = free_prompt(Lang::En, true);
        assert_eq!(f, format!("{}\n{FREE_MODE_BLOCK}", language_line(Lang::En)));
        assert!(!f.contains("study assistant inside Xexam"));
        assert!(NORMAL_MODE_BLOCK.len() < 120 && FREE_MODE_BLOCK.len() < 120);
        assert_eq!(
            suggest_prompt(Lang::Ja),
            "Rewrite the user's request as one short note about the user to remember for later chats. Write the note in Japanese. At most 200 characters. Reply with the note only."
        );
        assert!(suggest_prompt(Lang::Zh).contains("Write the note in Simplified Chinese."));
        assert!(!VERIFY_PROMPT.contains("Always reply in"));
    }

    #[test]
    fn prompt_with_memories() {
        let s = system_prompt(Lang::En, "2026-01-02", &[], false);
        assert!(s.contains("Today is 2026-01-02."));
        assert!(
            s.ends_with(&language_line(Lang::En)),
            "the language line is last"
        );
        assert!(!s.contains("Current mode"), "no mode line without a switch");
        assert!(!s.contains("remember these notes"));
        let mems = vec![
            "Prefers short answers.".to_string(),
            "x".repeat(3990),
            "Last".to_string(),
        ];
        let s = system_prompt(Lang::En, "2026-01-02", &mems, true);
        assert!(s.contains("\n\nThe user asked you to remember these notes"));
        assert!(s.contains("\n- Prefers short answers."));
        assert!(!s.contains("\n- Last"), "the third note does not fit");
    }

    #[test]
    fn history_budget_and_order() {
        let prev = [
            Prev {
                role: Role::User,
                content: "q1",
                status: "done",
                free: false,
            },
            Prev {
                role: Role::Assistant,
                content: "a1",
                status: "done",
                free: false,
            },
            Prev {
                role: Role::User,
                content: "q2",
                status: "done",
                free: false,
            },
            Prev {
                role: Role::Assistant,
                content: "",
                status: "error",
                free: false,
            },
            Prev {
                role: Role::User,
                content: "q3",
                status: "done",
                free: false,
            },
            Prev {
                role: Role::Assistant,
                content: "partial",
                status: "stopped",
                free: false,
            },
        ];
        let h = history(&prev, "new", 1000, false);
        let text: Vec<&str> = h.turns.iter().map(|t| t.content.as_str()).collect();
        assert_eq!(text, vec!["q1", "a1", "q2", "q3", "partial", "new"]);
        // budget: "new"(3) + "partial"(7) + "q3"(2) = 12; "q2" would make 14
        let h = history(&prev, "new", 13, false);
        let text: Vec<&str> = h.turns.iter().map(|t| t.content.as_str()).collect();
        assert_eq!(text, vec!["q3", "partial", "new"]);
        // an assistant message at the start is dropped
        let h = history(&prev, "new", 10, false);
        let text: Vec<&str> = h.turns.iter().map(|t| t.content.as_str()).collect();
        assert_eq!(text, vec!["new"]);
        assert_eq!(history(&[], "only", 1, false).turns.len(), 1);
        assert!(!history(&prev, "new", 1000, false).other_mode);
        assert!(history(&prev, "new", 1000, true).other_mode);
        // only the messages that are really sent count: "q1"/"a1" are cut off by the budget
        let mut mixed = [
            Prev {
                role: Role::User,
                content: "old",
                status: "done",
                free: true,
            },
            Prev {
                role: Role::Assistant,
                content: "ans",
                status: "done",
                free: true,
            },
            Prev {
                role: Role::User,
                content: "q",
                status: "done",
                free: false,
            },
            Prev {
                role: Role::Assistant,
                content: "a",
                status: "done",
                free: false,
            },
        ];
        assert!(history(&mixed, "new", 1000, false).other_mode);
        assert!(!history(&mixed, "new", 5, false).other_mode);
        mixed[0].free = false;
        mixed[1].free = false;
        assert!(!history(&mixed, "new", 1000, false).other_mode);
    }

    #[test]
    fn a_cut_depends_only_on_the_message_and_being_old() {
        let long_a = "a".repeat(900);
        let long_u = "u".repeat(2100);
        // the newest FULL_RECENT ranks are sent in full
        for rank in 0..FULL_RECENT {
            assert_eq!(shortened(Role::Assistant, &long_a, rank), long_a);
            assert_eq!(shortened(Role::User, &long_u, rank), long_u);
        }
        let a = shortened(Role::Assistant, &long_a, FULL_RECENT);
        assert_eq!(a, format!("{}…(shortened)", "a".repeat(800)));
        let u = shortened(Role::User, &long_u, 9);
        assert_eq!(u, format!("{}…", "u".repeat(2000)));
        // short and exactly-at-the-limit messages are untouched; multi-byte text is cut by characters
        assert_eq!(
            shortened(Role::Assistant, &"a".repeat(800), 7),
            "a".repeat(800)
        );
        assert_eq!(
            shortened(Role::User, &"u".repeat(2000), 7),
            "u".repeat(2000)
        );
        let jp = "あ".repeat(801);
        assert_eq!(
            shortened(Role::Assistant, &jp, 5),
            format!("{}…(shortened)", "あ".repeat(800))
        );
    }

    #[test]
    fn the_shortening_line_moves_in_steps_of_eight() {
        assert_eq!(STEP, 8);
        let lines: Vec<usize> = (0..=30).map(shortening_line).collect();
        // nothing is shortened up to 11 messages, then the line jumps by 8 every 8 messages
        assert!(lines[..=11].iter().all(|l| *l == 0));
        assert_eq!(lines[12], 8);
        assert_eq!(lines[19], 8);
        assert_eq!(lines[20], 16);
        assert_eq!(lines[28], 24);
        // never more than one move in 8 consecutive sizes
        for w in lines.windows(9) {
            assert!(w[8] - w[0] <= STEP);
        }
    }

    fn render(msgs: &[(Role, String)], budget: usize) -> Vec<(Role, String)> {
        let prev: Vec<Prev<'_>> = msgs
            .iter()
            .map(|(role, c)| Prev {
                role: *role,
                content: c,
                status: "done",
                free: false,
            })
            .collect();
        history(&prev, "NEW", budget, false)
            .turns
            .into_iter()
            .map(|t| (t.role, t.content))
            .collect()
    }

    /// Message `i` of a made-up chat: alternating roles, mixed lengths.
    fn chat(n: usize) -> Vec<(Role, String)> {
        (0..n)
            .map(|i| {
                let len = 40 + (i * 37) % 300;
                if i % 2 == 0 {
                    (Role::User, format!("q{i}:{}", "u".repeat(len * 7)))
                } else {
                    (Role::Assistant, format!("a{i}:{}", "a".repeat(len * 3)))
                }
            })
            .collect()
    }

    #[test]
    fn requests_extend_the_previous_one_between_two_moves() {
        let budget = 6_000;
        let mut prev_req: Option<Vec<(Role, String)>> = None;
        let mut moves = Vec::new();
        for n in 1..=60usize {
            let req = render(&chat(n), budget);
            // never over budget; the first turn is a user message
            let chars: usize = req.iter().map(|t| char_len(&t.1)).sum();
            assert!(chars <= budget || req.len() == 1, "n={n}: {chars}");
            assert_eq!(req[0].0, Role::User, "n={n}");
            assert_eq!(req.last().map(|t| t.1.as_str()), Some("NEW"));
            if let Some(p) = &prev_req {
                // the previous request without its new message is a prefix of this request
                let old = &p[..p.len() - 1];
                let is_prefix = req.len() > old.len() && req[..old.len()] == *old;
                if !is_prefix {
                    moves.push(n);
                }
            }
            prev_req = Some(req);
        }
        // the shortening line B and the window start S each move once per 8 messages (4 turns): so at most two moves in
        // any 8 consecutive sizes, and each of the two kinds alone moves at most once
        // (the first sizes are the start-up of the window: no multiple of 8 fits yet, so today's rule applies)
        let steady: Vec<usize> = moves.iter().copied().filter(|n| *n >= 16).collect();
        for w in steady.windows(3) {
            assert!(
                w[2] - w[0] >= STEP,
                "three moves within 8 messages: {moves:?}"
            );
        }
        assert!(
            steady.len() >= 4,
            "the window and the line do move: {moves:?}"
        );
        assert!(
            moves.len() <= 2 * (60 / STEP) + 2,
            "too many moves: {moves:?}"
        );
    }

    #[test]
    fn the_window_start_is_a_multiple_of_eight_and_the_history_is_stable() {
        // 30 messages of 1000 characters, budget for about 12 of them: the start is the smallest multiple of 8 that fits
        let msgs: Vec<(Role, String)> = (0..30)
            .map(|i| {
                (
                    if i % 2 == 0 {
                        Role::User
                    } else {
                        Role::Assistant
                    },
                    format!("{i:02}{}", "x".repeat(998)),
                )
            })
            .collect();
        let req = render(&msgs[..24], 14_000);
        // line = floor(20/8)*8 = 16: messages 0..16 are cut; the window starts where the rest fits
        let first = &req[0].1;
        let start: usize = first[..2].parse().expect("index");
        assert_eq!(start % STEP, 0, "window start {start}");
        // the same chat with 3 more messages: same start (no move), the request only grew
        let longer = render(&msgs[..27], 14_000);
        assert_eq!(longer[0].1, req[0].1);
        // the shortening line is 16 for 24 messages: message 15 is cut, message 16 is in full
        let cut: Vec<&(Role, String)> = req.iter().filter(|t| t.1.starts_with("15")).collect();
        if let Some(t) = cut.first() {
            assert!(t.1.ends_with("…(shortened)") || t.1.ends_with('…') || char_len(&t.1) <= 800);
        }
        let full = req.iter().find(|t| t.1.starts_with("16")).expect("m16");
        assert_eq!(char_len(&full.1), 1000);
        // nothing fits any multiple: today's rule, newest first
        let tiny = render(&msgs[..24], 2_300);
        assert!(tiny.len() <= 3);
        assert_eq!(tiny.last().map(|t| t.1.as_str()), Some("NEW"));
    }
}
