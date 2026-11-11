//! A small server-sent-events parser (4.6): accepts `\n`, `\r\n` and `\r` line ends (also split across chunks), joins
//! multi-line `data:`, ignores comment lines (`:`), reads `event:` names, dispatches on a blank line, and refuses an
//! event larger than the limit.

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` name, `message` when none was given
    pub name: String,
    pub data: String,
}

/// The event (or a single line) grew beyond the limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLarge;

pub struct SseParser {
    max: usize,
    line: Vec<u8>,
    /// The previous chunk ended with `\r`: a `\n` at the start of the next one belongs to it
    after_cr: bool,
    name: Option<String>,
    data: String,
    has_data: bool,
    size: usize,
}

impl SseParser {
    pub fn new(max: usize) -> SseParser {
        SseParser {
            max,
            line: Vec::new(),
            after_cr: false,
            name: None,
            data: String::new(),
            has_data: false,
            size: 0,
        }
    }

    /// Feeds bytes; returns the events completed by them.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, TooLarge> {
        let mut out = Vec::new();
        for &b in bytes {
            if self.after_cr {
                self.after_cr = false;
                if b == b'\n' {
                    continue;
                }
            }
            match b {
                b'\n' => self.end_line(&mut out)?,
                b'\r' => {
                    self.end_line(&mut out)?;
                    self.after_cr = true;
                }
                _ => {
                    self.line.push(b);
                    if self.size + self.line.len() > self.max {
                        return Err(TooLarge);
                    }
                }
            }
        }
        Ok(out)
    }

    /// The body ended: a last line without a line end and a pending event with data are dispatched.
    pub fn finish(&mut self) -> Result<Vec<SseEvent>, TooLarge> {
        let mut out = Vec::new();
        if !self.line.is_empty() {
            self.end_line(&mut out)?;
        }
        self.dispatch(&mut out);
        Ok(out)
    }

    fn end_line(&mut self, out: &mut Vec<SseEvent>) -> Result<(), TooLarge> {
        let line = std::mem::take(&mut self.line);
        if line.is_empty() {
            self.dispatch(out);
            return Ok(());
        }
        if line.first() == Some(&b':') {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&line);
        let (field, value) = match text.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (text.as_ref(), ""),
        };
        match field {
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
                self.size = self.size.saturating_add(value.len() + 1);
                if self.size > self.max {
                    return Err(TooLarge);
                }
            }
            "event" => self.name = Some(value.to_string()),
            _ => {}
        }
        Ok(())
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        let name = self.name.take();
        if self.has_data {
            out.push(SseEvent {
                name: name
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| "message".to_string()),
                data: std::mem::take(&mut self.data),
            });
        }
        self.data.clear();
        self.has_data = false;
        self.size = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(p: &mut SseParser, chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut out = Vec::new();
        for c in chunks {
            out.extend(p.push(c).expect("push"));
        }
        out.extend(p.finish().expect("finish"));
        out
    }

    #[test]
    fn line_ends_comments_and_names() {
        let mut p = SseParser::new(1 << 20);
        let ev = all(
            &mut p,
            &[
                b": keep-alive\n\nevent: ping\r\ndata: {}\r\n\r\ndata: a\rdata: b\r\r",
                b"data: [DONE]\n\n",
            ],
        );
        assert_eq!(
            ev,
            vec![
                SseEvent {
                    name: "ping".into(),
                    data: "{}".into()
                },
                SseEvent {
                    name: "message".into(),
                    data: "a\nb".into()
                },
                SseEvent {
                    name: "message".into(),
                    data: "[DONE]".into()
                },
            ]
        );
    }

    #[test]
    fn split_anywhere_gives_the_same_events() {
        let body: &[u8] =
            "event: content_block_delta\r\ndata: {\"t\":\"héllo\"}\r\n\r\n: c\n\ndata:x\n\n"
                .as_bytes();
        let mut whole = SseParser::new(1 << 20);
        let expect = all(&mut whole, &[body]);
        assert_eq!(expect.len(), 2);
        for cut in 0..body.len() {
            let (a, b) = body.split_at(cut);
            let mut p = SseParser::new(1 << 20);
            assert_eq!(all(&mut p, &[a, b]), expect, "cut at {cut}");
        }
        // byte by byte
        let mut p = SseParser::new(1 << 20);
        let pieces: Vec<&[u8]> = body.chunks(1).collect();
        assert_eq!(all(&mut p, &pieces), expect);
    }

    #[test]
    fn an_event_without_data_is_not_dispatched_and_oversize_fails() {
        let mut p = SseParser::new(1 << 20);
        assert!(all(&mut p, &[b"event: x\n\n"]).is_empty());
        let mut p = SseParser::new(16);
        assert_eq!(p.push(b"data: 0123456789abcdefghij\n"), Err(TooLarge));
        let mut p = SseParser::new(16);
        assert_eq!(
            p.push(b"data: 01234\ndata: 56789\ndata: abc\n"),
            Err(TooLarge)
        );
        let mut p = SseParser::new(16);
        assert_eq!(p.push(&[b'x'; 20]), Err(TooLarge));
    }

    #[test]
    fn last_event_without_blank_line_is_kept() {
        let mut p = SseParser::new(1 << 20);
        assert_eq!(
            all(&mut p, &[b"data: tail"]),
            vec![SseEvent {
                name: "message".into(),
                data: "tail".into()
            }]
        );
    }
}
