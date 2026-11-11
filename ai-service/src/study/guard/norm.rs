//! Normalisation for checking only; released text is always the raw text.
//!
//! Full-width ASCII forms (U+FF01..U+FF5E) become ASCII, U+3000 a space,
//! curly apostrophes a plain one, whitespace runs one space, and a run with
//! two or more line breaks one `'\n'` (a sentence end). A single line break
//! is remembered as a flag on the next character (`ls`, "line start").

/// One normalised character.
#[derive(Clone, Copy, Debug)]
pub struct Nc {
    /// The normalised character (case kept).
    pub ch: char,
    /// Lowercase of `ch` (first code point of its lowercase form).
    pub lc: char,
    /// Byte offset of the raw character in the whole raw stream.
    pub raw: usize,
    /// True when this character starts a line.
    pub ls: bool,
}

fn map_char(c: char) -> char {
    match c {
        '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
        '\u{3000}' => ' ',
        '\u{2018}' | '\u{2019}' => '\'',
        _ => c,
    }
}

fn lower(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Incremental normaliser; the whitespace state is carried across pieces.
#[derive(Debug)]
pub struct Normaliser {
    in_ws: bool,
    ws_newlines: usize,
    at_line_start: bool,
}

impl Default for Normaliser {
    fn default() -> Self {
        Normaliser {
            in_ws: false,
            ws_newlines: 0,
            at_line_start: true,
        }
    }
}

impl Normaliser {
    /// Normalises `piece` (whose first byte is at raw offset `base`) onto `out`.
    pub fn feed(&mut self, piece: &str, base: usize, out: &mut Vec<Nc>) {
        for (idx, c) in piece.char_indices() {
            let raw = base + idx;
            let c = map_char(c);
            if c.is_whitespace() {
                if !self.in_ws {
                    self.in_ws = true;
                    self.ws_newlines = 0;
                    out.push(Nc {
                        ch: ' ',
                        lc: ' ',
                        raw,
                        ls: false,
                    });
                }
                if c == '\n' {
                    self.ws_newlines += 1;
                    self.at_line_start = true;
                    if self.ws_newlines == 2 {
                        if let Some(last) = out.last_mut() {
                            last.ch = '\n';
                            last.lc = '\n';
                        }
                    }
                }
            } else {
                self.in_ws = false;
                out.push(Nc {
                    ch: c,
                    lc: lower(c),
                    raw,
                    ls: self.at_line_start,
                });
                self.at_line_start = false;
            }
        }
    }
}

/// Normalises a whole text.
pub fn normalize(text: &str) -> Vec<Nc> {
    let mut out = Vec::with_capacity(text.len());
    Normaliser::default().feed(text, 0, &mut out);
    out
}

/// Normalised, lowercased and trimmed characters of a checked text.
pub fn norm_lower(text: &str) -> Vec<char> {
    let mut v: Vec<char> = normalize(text).iter().map(|n| n.lc).collect();
    while matches!(v.last(), Some(' ' | '\n')) {
        v.pop();
    }
    let lead = v.iter().take_while(|c| matches!(c, ' ' | '\n')).count();
    v.drain(..lead);
    v
}

/// Sentence end of the normalised text.
pub fn is_sentence_end(c: char) -> bool {
    matches!(c, '。' | '.' | '!' | '?' | '\n')
}

/// CJK (Han and kana) character.
pub fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}'
        | '\u{F900}'..='\u{FAFF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> String {
        normalize(s).iter().map(|n| n.ch).collect()
    }

    #[test]
    fn full_width_and_spaces() {
        assert_eq!(text("ＡＢ！　ｃ"), "AB! c");
    }

    #[test]
    fn whitespace_runs_collapse() {
        assert_eq!(text("a \t\r\n  b"), "a b");
    }

    #[test]
    fn blank_line_is_a_sentence_end() {
        assert_eq!(text("a\n\nb"), "a\nb");
        assert_eq!(text("a \n \n \nb"), "a\nb");
    }

    #[test]
    fn single_break_marks_line_start() {
        let n = normalize("a\nb c");
        assert!(n[0].ls);
        assert!(n[2].ls);
        assert!(!n[4].ls);
    }

    #[test]
    fn whitespace_state_crosses_pieces() {
        let mut out = Vec::new();
        let mut nm = Normaliser::default();
        nm.feed("a \n", 0, &mut out);
        nm.feed("\n b", 3, &mut out);
        let s: String = out.iter().map(|n| n.ch).collect();
        assert_eq!(s, "a\nb");
        assert_eq!(out[2].raw, 5);
    }

    #[test]
    fn raw_offsets_are_byte_offsets() {
        let n = normalize("あB");
        assert_eq!(n[0].raw, 0);
        assert_eq!(n[1].raw, 3);
    }

    #[test]
    fn checked_texts_are_trimmed_and_lowercased() {
        assert_eq!(
            norm_lower("  Ｍercury \n\n"),
            "mercury".chars().collect::<Vec<_>>()
        );
    }
}
