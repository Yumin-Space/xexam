//! The system prompt and the history sent with a message (spec 4.5; reply language and free mode: round 2, 5.2 / 7).
use crate::limits::MEMORY_PROMPT_CHARS;
use crate::provider::{Role, Turn};
use crate::util::char_len;

/// The reply language: the language chosen in the user's Xexam settings, sent by the frontend as `lang`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The text of a history message as sent: the newest [`FULL_RECENT`] messages (`rank` 0 = newest) in full, older ones
/// cut to a fixed prefix. The cut depends only on the message and on being old, so the same message always gives the
/// same text and the start of the request stays identical between requests (prompt caching).
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

/// The messages sent upstream: earlier `done` / `stopped` messages with text, the older ones shortened
/// ([`shortened`]), newest first while the running total (the new message included, the shortened length counted)
/// stays within `budget` characters, then in order, ending with the new user message. A message that does not fit
/// ends the history (never cut in half); a leading assistant message is dropped.
pub fn history(prev: &[Prev<'_>], new_message: &str, budget: usize, free: bool) -> History {
    let mut total = char_len(new_message);
    let mut picked: Vec<(Turn, bool)> = Vec::new();
    let usable = prev
        .iter()
        .rev()
        .filter(|p| (p.status == "done" || p.status == "stopped") && !p.content.is_empty());
    for (rank, p) in usable.enumerate() {
        let content = shortened(p.role, p.content, rank);
        let n = char_len(&content);
        if total + n > budget {
            break;
        }
        total += n;
        picked.push((
            Turn {
                role: p.role,
                content,
            },
            p.free,
        ));
    }
    picked.reverse();
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
    fn old_messages_are_cut_deterministically() {
        let long_a = "a".repeat(900);
        let long_u = "u".repeat(2100);
        // the newest FULL_RECENT messages are sent in full
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
        // in a history: 6 long messages, the two oldest are cut; the same cut on the next request
        let mk = |n: usize| -> Vec<(Role, String)> {
            (0..n)
                .map(|i| {
                    if i % 2 == 0 {
                        (Role::User, long_u.clone())
                    } else {
                        (Role::Assistant, long_a.clone())
                    }
                })
                .collect()
        };
        let run = |msgs: &[(Role, String)]| {
            let prev: Vec<Prev<'_>> = msgs
                .iter()
                .map(|(role, c)| Prev {
                    role: *role,
                    content: c,
                    status: "done",
                    free: false,
                })
                .collect();
            history(&prev, "q", 1_000_000, false).turns
        };
        let first = run(&mk(6));
        assert_eq!(first[0].content.chars().count(), 2001);
        assert!(first[0].content.ends_with('…'));
        assert_eq!(
            first[1].content.chars().count(),
            800 + "…(shortened)".chars().count()
        );
        assert_eq!(
            first[2].content.chars().count(),
            2100,
            "the newest four are in full"
        );
        // two more messages arrive (a new question and its answer): the cut of the old ones does not change
        let second = run(&mk(8));
        assert_eq!(second[2].content, first[0].content);
        assert_eq!(second[3].content, first[1].content);
        // the budget counts the shortened length and drops the oldest first: 6001 (the newest four and "q") + 812
        // (the shortened answer) fit, the shortened 2001-character question does not
        let msgs = mk(6);
        let prev: Vec<Prev<'_>> = msgs
            .iter()
            .map(|(role, c)| Prev {
                role: *role,
                content: c,
                status: "done",
                free: false,
            })
            .collect();
        let h = history(&prev, "q", 6900, false);
        // the shortened answer fits but would lead the history: dropped
        assert_eq!(h.turns.len(), 5);
        assert_eq!(h.turns[0].content.chars().count(), 2100);
    }
}
