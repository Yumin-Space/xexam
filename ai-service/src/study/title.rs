//! Chat titles. The stored title holds only the time; the viewer adds the localised suffix
//! (`<start>-练习`, `<start>-考试`, `<record time>-AI总结`), spec 5.2 and 7.10.

/// The kind of a chat (`chats.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatKind {
    Chat,
    Practice,
    Exam,
    Summary,
}

/// The frontend i18n key of the title suffix; `None` for an ordinary AI Chat chat.
pub fn suffix_key(kind: ChatKind) -> Option<&'static str> {
    match kind {
        ChatKind::Chat => None,
        ChatKind::Practice => Some("aiplus.chat.suffix.practice"),
        ChatKind::Exam => Some("aiplus.chat.suffix.exam"),
        ChatKind::Summary => Some("aiplus.chat.suffix.summary"),
    }
}

/// The `ref` of a summary chat: `r<recordId>@<record at>`, e.g. `r42@2026-11-11 21:58`.
pub fn summary_ref(record_id: i64, at: &str) -> String {
    format!("r{record_id}@{at}")
}

/// Rebuilds `YYYY-MM-DD HH:MM` from the 12 digits of a saved-summary query (`202611112158`); anything else is `None`.
pub fn at_digits_to_at(d: &str) -> Option<String> {
    if d.len() != 12 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!(
        "{}-{}-{} {}:{}",
        &d[0..4],
        &d[4..6],
        &d[6..8],
        &d[8..10],
        &d[10..12]
    ))
}

/// A browser-supplied start time is trusted only inside a sane window: later than `now + 300 s` or earlier than
/// `now - 400 days` falls back to `now` (spec 5.2).
pub fn clamp_started_at(started: i64, now: i64) -> i64 {
    if started > now + 300 || started < now - 400 * 86_400 {
        now
    } else {
        started
    }
}
