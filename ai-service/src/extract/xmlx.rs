//! Streaming XML for the Office parts (IX-3, SEC-4): no tree, local names only, a DOCTYPE refused, entities never
//! expanded (the predefined ones and character references are resolved, any other entity reference is `Corrupt`), depth,
//! attributes and event counts capped per part and in total, processing instructions and comments ignored; `xml:base`
//! and XInclude are never followed (nothing here fetches anything).
use super::{ExtractError, Limits};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::borrow::Cow;

/// Events counted across all parts of one extraction (SEC-4).
#[derive(Debug, Default)]
pub struct XmlBudget {
    pub total: u64,
}

/// A start tag: the local name and the attributes (qualified name, unescaped value).
#[derive(Debug, Clone, Default)]
pub struct Tag {
    pub name: String,
    pub attrs: Vec<(String, String)>,
}

impl Tag {
    /// The value of the attribute with this local name (`r:id` is `id`). A prefixed attribute wins over an unprefixed
    /// one with the same local name (`<p:sldId id="256" r:id="rId2"/>` answers `rId2` for `id`).
    pub fn attr(&self, local: &str) -> Option<&str> {
        let found = |prefixed: bool| {
            self.attrs.iter().find(|(k, _)| match k.rsplit_once(':') {
                Some((_, l)) => prefixed && l == local,
                None => !prefixed && k == local,
            })
        };
        found(true)
            .or_else(|| found(false))
            .map(|(_, v)| v.as_str())
    }
}

/// What the walker hands to the callback. An empty element is a `Start` followed by an `End`.
#[derive(Debug)]
pub enum Ev<'a> {
    Start(&'a Tag),
    End(&'a str),
    Text(&'a str),
}

/// The part's text: UTF-8 with or without a BOM, UTF-16 with a BOM transcoded; anything else is `Corrupt`.
pub fn decode_part(bytes: &[u8]) -> Result<Cow<'_, str>, ExtractError> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return std::str::from_utf8(rest)
            .map(Cow::Borrowed)
            .map_err(|_| ExtractError::Corrupt);
    }
    for (bom, big) in [([0xFF, 0xFE], false), ([0xFE, 0xFF], true)] {
        if let Some(rest) = bytes.strip_prefix(&bom) {
            let (pairs, tail) = rest.as_chunks::<2>();
            if !tail.is_empty() {
                return Err(ExtractError::Corrupt);
            }
            let units: Vec<u16> = pairs
                .iter()
                .map(|p| {
                    if big {
                        u16::from_be_bytes(*p)
                    } else {
                        u16::from_le_bytes(*p)
                    }
                })
                .collect();
            return String::from_utf16(&units)
                .map(Cow::Owned)
                .map_err(|_| ExtractError::Corrupt);
        }
    }
    std::str::from_utf8(bytes)
        .map(Cow::Borrowed)
        .map_err(|_| ExtractError::Corrupt)
}

fn predefined(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => None,
    }
}

/// Walks one part. Every refusal is an `ExtractError`; the callback may stop early with its own error.
pub fn walk<F>(
    bytes: &[u8],
    limits: &Limits,
    budget: &mut XmlBudget,
    mut f: F,
) -> Result<(), ExtractError>
where
    F: FnMut(Ev<'_>) -> Result<(), ExtractError>,
{
    let text = decode_part(bytes)?;
    let mut reader = Reader::from_str(&text);
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0usize;
    let mut events = 0u64;
    let mut tag = Tag::default();
    loop {
        events += 1;
        budget.total += 1;
        if events > limits.xml_events_per_part || budget.total > limits.xml_events_total {
            return Err(ExtractError::TooComplex);
        }
        if events.is_multiple_of(4096) {
            limits.check_deadline()?;
        }
        let ev = reader.read_event().map_err(|_| ExtractError::Corrupt)?;
        match ev {
            Event::Start(e) => {
                depth += 1;
                if depth > limits.xml_depth {
                    return Err(ExtractError::TooComplex);
                }
                tag.name.clear();
                tag.name.push_str(e.local_name().as_ref());
                tag.attrs.clear();
                for a in e.attributes() {
                    let a = a.map_err(|_| ExtractError::Corrupt)?;
                    if tag.attrs.len() >= limits.xml_attrs {
                        return Err(ExtractError::TooComplex);
                    }
                    let value = a
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|_| ExtractError::Corrupt)?
                        .into_owned();
                    tag.attrs.push((a.key.as_ref().to_string(), value));
                }
                f(Ev::Start(&tag))?;
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                let name = e.local_name();
                f(Ev::End(name.as_ref()))?;
            }
            Event::Empty(_) => return Err(ExtractError::Corrupt), // expanded by the reader
            Event::Text(t) => {
                let s = t.xml10_content();
                if !s.is_empty() {
                    f(Ev::Text(&s))?;
                }
            }
            Event::CData(c) => {
                let s = c.xml10_content();
                if !s.is_empty() {
                    f(Ev::Text(&s))?;
                }
            }
            Event::GeneralRef(r) => {
                let c = match r.resolve_char_ref().map_err(|_| ExtractError::Corrupt)? {
                    Some(c) => c,
                    None => predefined(&r).ok_or(ExtractError::Corrupt)?,
                };
                let mut buf = [0u8; 4];
                f(Ev::Text(c.encode_utf8(&mut buf)))?;
            }
            Event::DocType(_) => return Err(ExtractError::TooComplex),
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
            Event::Eof => {
                return if depth == 0 {
                    Ok(())
                } else {
                    Err(ExtractError::Corrupt)
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::testgen;

    fn collect(xml: &[u8], limits: &Limits) -> Result<Vec<String>, ExtractError> {
        let mut out = Vec::new();
        let mut b = XmlBudget::default();
        walk(xml, limits, &mut b, |e| {
            match e {
                Ev::Start(t) => out.push(format!(
                    "<{}{}",
                    t.name,
                    t.attrs
                        .iter()
                        .map(|(k, v)| format!(
                            " {}={v}",
                            k.rsplit_once(':').map_or(k.as_str(), |(_, l)| l)
                        ))
                        .collect::<String>()
                )),
                Ev::End(n) => out.push(format!("</{n}")),
                Ev::Text(s) => out.push(format!("'{s}")),
            }
            Ok(())
        })?;
        Ok(out)
    }

    #[test]
    fn local_names_attributes_text_and_empty_elements() {
        let xml = br#"<?xml version="1.0"?><w:p xmlns:w="u" w:val="a&amp;b"><w:t>x &lt; y &#65;</w:t><w:br/></w:p>"#;
        let ev = collect(xml, &Limits::default()).unwrap();
        assert!(ev[0].ends_with("val=a&b"), "{}", ev[0]);
        assert!(ev.contains(&"'x ".to_string()));
        assert!(ev.contains(&"'<".to_string()));
        assert!(ev.contains(&"'A".to_string()));
        assert_eq!(&ev[ev.len() - 3..], ["<br", "</br", "</p"]);
    }

    #[test]
    fn a_doctype_with_entities_is_refused_and_not_expanded() {
        let start = std::time::Instant::now();
        assert_eq!(
            collect(&testgen::xml_doctype_entities(), &Limits::default()),
            Err(ExtractError::TooComplex)
        );
        assert!(start.elapsed().as_secs() < 2);
    }

    #[test]
    fn an_unknown_entity_is_corrupt() {
        assert_eq!(
            collect(b"<a>&nbsp;</a>", &Limits::default()),
            Err(ExtractError::Corrupt)
        );
    }

    #[test]
    fn depth_attributes_and_events_are_capped() {
        let l = Limits::small();
        assert!(collect(&testgen::xml_deep(16), &l).is_ok());
        assert_eq!(
            collect(&testgen::xml_deep(17), &l),
            Err(ExtractError::TooComplex)
        );
        let many: String = (0..17).map(|i| format!(" a{i}=\"1\"")).collect();
        assert_eq!(
            collect(format!("<x{many}/>").as_bytes(), &l),
            Err(ExtractError::TooComplex)
        );
        let busy = format!("<r>{}</r>", "<i/>".repeat(3000));
        assert_eq!(collect(busy.as_bytes(), &l), Err(ExtractError::TooComplex));
    }

    #[test]
    fn the_total_budget_spans_parts() {
        let l = Limits {
            xml_events_total: 10,
            ..Limits::default()
        };
        let mut b = XmlBudget::default();
        let part = b"<a><b/><b/></a>";
        assert!(walk(part, &l, &mut b, |_| Ok(())).is_ok());
        assert_eq!(
            walk(part, &l, &mut b, |_| Ok(())),
            Err(ExtractError::TooComplex)
        );
    }

    #[test]
    fn malformed_input_is_corrupt_and_comments_and_pis_are_ignored() {
        assert_eq!(
            collect(b"<a><b></a>", &Limits::default()),
            Err(ExtractError::Corrupt)
        );
        assert_eq!(
            collect(b"<a>", &Limits::default()),
            Err(ExtractError::Corrupt)
        );
        let ev = collect(b"<a><!-- c --><?pi x?>t</a>", &Limits::default()).unwrap();
        assert_eq!(ev, vec!["<a", "'t", "</a"]);
    }

    #[test]
    fn utf16_and_bom_parts_are_read() {
        let u16le = testgen::utf16le_bom("<a>caf\u{e9} 世界</a>");
        let ev = collect(&u16le, &Limits::default()).unwrap();
        assert_eq!(ev[1], "'café 世界");
        let u8bom = testgen::utf8_bom("<a>x</a>");
        assert!(collect(&u8bom, &Limits::default()).is_ok());
        // a legacy encoding without a BOM is not guessed
        assert_eq!(
            collect(
                &[b'<', b'a', b'>', 0xE9, b'<', b'/', b'a', b'>'],
                &Limits::default()
            ),
            Err(ExtractError::Corrupt)
        );
    }

    #[test]
    fn the_deadline_is_checked() {
        let l = Limits {
            deadline: Some(std::time::Instant::now() - std::time::Duration::from_secs(1)),
            ..Limits::default()
        };
        let busy = format!("<r>{}</r>", "<i/>".repeat(3000));
        assert_eq!(collect(busy.as_bytes(), &l), Err(ExtractError::Timeout));
    }
}
