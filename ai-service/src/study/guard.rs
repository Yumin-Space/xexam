//! The streaming hint guard: hold-back buffer, windowed scans, release.

use crate::study::model::{Solution, StudyQuestion};
use lists::{HINT_MAX_CHARS, SCAN_EVERY, WINDOW_SLACK};
use norm::{normalize, Nc, Normaliser};
use rules::Rules;

pub mod lists;
mod norm;
mod rules;

/// Normalised characters kept behind the release point as lookbehind context.
const CONTEXT: usize = 96;

/// What the caller may do with the text it just fed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Text that may be sent and stored now (possibly empty).
    Release(String),
    /// The hint gives the answer away: withhold everything, store nothing.
    Leak,
}

/// Streaming guard for one hint generation.
///
/// Persistence rule: the raw text lives only in here; callers store and show
/// only what [`Verdict::Release`] hands out.
pub struct HintGuard {
    rules: Rules,
    hold: usize,
    norm: Normaliser,
    /// Normalised characters: context, then unreleased text.
    buf: Vec<Nc>,
    /// Index in `buf` of the first unreleased character.
    rel: usize,
    /// Raw text not released yet.
    raw: String,
    /// Raw stream offset of `raw[0]`.
    raw_base: usize,
    /// Raw bytes fed so far.
    raw_total: usize,
    /// Raw characters fed so far (for the size limit).
    raw_chars: usize,
    /// Normalised characters not yet covered by a scan.
    pending: usize,
    /// Incomplete UTF-8 sequence at the end of the last byte piece.
    carry: Vec<u8>,
    truncated: bool,
    leaked: bool,
    finished: bool,
    released_chars: usize,
    scanned: u64,
}

impl HintGuard {
    /// The guard is the only hint code that sees the solution.
    pub fn new(q: &StudyQuestion, sol: &Solution) -> HintGuard {
        let rules = Rules::new(q, &sol.key_texts);
        HintGuard {
            hold: rules.hold(),
            rules,
            norm: Normaliser::default(),
            buf: Vec::new(),
            rel: 0,
            raw: String::new(),
            raw_base: 0,
            raw_total: 0,
            raw_chars: 0,
            pending: 0,
            carry: Vec::new(),
            truncated: false,
            leaked: false,
            finished: false,
            released_chars: 0,
            scanned: 0,
        }
    }

    /// Feeds a text piece; returns the text that may be sent now.
    pub fn push(&mut self, piece: &str) -> Verdict {
        if self.leaked {
            return Verdict::Leak;
        }
        if self.finished || self.truncated {
            return Verdict::Release(String::new());
        }
        self.feed(piece)
    }

    /// Feeds raw upstream bytes; a UTF-8 sequence split across pieces is
    /// completed by the next piece, invalid bytes become U+FFFD.
    pub fn push_bytes(&mut self, bytes: &[u8]) -> Verdict {
        if self.leaked {
            return Verdict::Leak;
        }
        if self.finished || self.truncated {
            return Verdict::Release(String::new());
        }
        let mut data = std::mem::take(&mut self.carry);
        data.extend_from_slice(bytes);
        let mut text = String::new();
        let mut rest: &[u8] = &data;
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    text.push_str(s);
                    break;
                }
                Err(e) => {
                    let (good, bad) = rest.split_at(e.valid_up_to());
                    text.push_str(std::str::from_utf8(good).unwrap_or_default());
                    match e.error_len() {
                        Some(n) => {
                            text.push('\u{FFFD}');
                            rest = &bad[n..];
                        }
                        None => {
                            self.carry = bad.to_vec();
                            break;
                        }
                    }
                }
            }
        }
        self.feed(&text)
    }

    /// The final check; returns the rest of the text. Call it on every outcome
    /// (done, failed or cancelled) before the final transaction.
    pub fn finish(&mut self) -> Verdict {
        if self.leaked {
            return Verdict::Leak;
        }
        if self.finished {
            return Verdict::Release(String::new());
        }
        let mut lead = String::new();
        if !self.carry.is_empty() && !self.truncated {
            self.carry.clear();
            match self.feed("\u{FFFD}") {
                Verdict::Leak => return Verdict::Leak,
                Verdict::Release(s) => lead = s,
            }
        }
        self.finished = true;
        match self.scan_and_release(true) {
            Verdict::Leak => Verdict::Leak,
            Verdict::Release(s) => {
                lead.push_str(&s);
                Verdict::Release(lead)
            }
        }
    }

    /// True once the raw hint reached [`HINT_MAX_CHARS`]; the caller drops the
    /// upstream reader, calls [`HintGuard::finish`] and ends as truncated.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// Characters released so far.
    pub fn released_chars(&self) -> usize {
        self.released_chars
    }

    /// Debug counter: normalised characters scanned so far.
    pub fn scanned_chars(&self) -> u64 {
        self.scanned
    }

    /// The hold-back length in normalised characters.
    pub fn hold(&self) -> usize {
        self.hold
    }

    fn feed(&mut self, piece: &str) -> Verdict {
        let mut piece = piece;
        let room = HINT_MAX_CHARS - self.raw_chars;
        if piece.chars().count() > room {
            if let Some((cut, _)) = piece.char_indices().nth(room) {
                piece = &piece[..cut];
            }
            self.truncated = true;
        }
        self.raw_chars += piece.chars().count();
        let before = self.buf.len();
        self.norm.feed(piece, self.raw_total, &mut self.buf);
        self.raw.push_str(piece);
        self.raw_total += piece.len();
        self.pending += self.buf.len() - before;
        if self.pending >= SCAN_EVERY {
            return self.scan_and_release(false);
        }
        Verdict::Release(String::new())
    }

    fn scan_and_release(&mut self, fin: bool) -> Verdict {
        let len = self.buf.len();
        let window = len.min(self.hold + self.pending + WINDOW_SLACK);
        let from = len - window;
        self.scanned += window as u64;
        if self.rules.leaks(&self.buf, from, fin) {
            self.leaked = true;
            self.raw.clear();
            return Verdict::Leak;
        }
        self.pending = 0;
        let cut = if fin {
            len
        } else {
            len.saturating_sub(self.hold).max(self.rel)
        };
        if cut == self.rel && !fin {
            return Verdict::Release(String::new());
        }
        let raw_cut = if cut == len {
            self.raw_total
        } else {
            self.buf[cut].raw
        };
        let out: String = self.raw.drain(..raw_cut - self.raw_base).collect();
        self.raw_base = raw_cut;
        self.rel = cut;
        self.released_chars += out.chars().count();
        let drop = self.rel.saturating_sub(CONTEXT);
        self.buf.drain(..drop);
        self.rel -= drop;
        Verdict::Release(out)
    }
}

/// The same rules on a whole text; pure.
pub fn leaks(text: &str, q: &StudyQuestion, key_texts: &[String]) -> bool {
    let rules = Rules::new(q, key_texts);
    rules.leaks(&normalize(text), 0, true)
}
