//! XML 层：把 quick-xml 的事件流变成一棵与 Python `ElementTree` 语义等价的小树。
//!
//! 与 ET 一致的地方：命名空间按 URI 匹配（前缀无关）；`text` 只取第一个子元素之前的文本；注释 / 处理指令丢弃；
//! CDATA 并入文本；字面 `\r\n` / `\r` 规范化为 `\n`（字符引用 `&#13;` 保留为 CR）；属性值里的空白字符变空格。
//! 比 ET 严格的地方（有意）：任何 DOCTYPE 都拒绝（不展开实体）；元素深度、节点数有上限。
//! 编码：UTF-8（含 BOM）、UTF-16LE/BE（含或不含 BOM，先转码）、声明为 ISO-8859-1 的单字节文本。
use std::borrow::Cow;

use quick_xml::events::{BytesStart, Event};
use quick_xml::name::ResolveResult;
use quick_xml::{NsReader, XmlVersion};

/// 元素深度上限（正常 OOXML 不到 16）。
pub(super) const MAX_DEPTH: usize = 64;
/// 单个 XML 部件的节点数上限（整棵树在内存里，约 150 字节 / 节点）。
pub(super) const MAX_NODES: usize = 1_000_000;
/// 一个元素上最多的属性个数（含命名空间声明）。真实的 .xlsx 里一个元素只有几个到十几个；`MAX_NODES` 只数元素，限制不了属性。
pub(super) const MAX_ATTRS_PER_ELEMENT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum XmlError {
    /// 不是格式良好的 XML（对应 ET 的 `ParseError`）
    Malformed,
    /// 出现 DOCTYPE（含实体声明）：一律拒绝
    Doctype,
    /// 超过深度 / 节点数上限
    TooComplex,
    /// 超过截止时间
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Attr {
    pub ns: Option<String>,
    pub local: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Element {
    pub ns: Option<String>,
    pub local: String,
    pub attrs: Vec<Attr>,
    /// 第一个子元素之前的文本；`None` = 没有文本（ET 的 `text is None`）
    pub text: Option<String>,
    pub children: Vec<Element>,
}

impl Element {
    pub(super) fn is(&self, ns: &str, local: &str) -> bool {
        self.ns.as_deref() == Some(ns) && self.local == local
    }

    /// 属性：`ns = None` 取无命名空间的属性（如 `r`、`t`、`name`）。
    pub(super) fn attr(&self, ns: Option<&str>, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.as_deref() == ns && a.local == local)
            .map(|a| a.value.as_str())
    }

    /// 直接子元素里第一个匹配的（ET `find`）。
    pub(super) fn find(&self, ns: &str, local: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.is(ns, local))
    }

    /// 直接子元素里全部匹配的（ET `findall`）。
    pub(super) fn find_all<'a>(
        &'a self,
        ns: &'a str,
        local: &'a str,
    ) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.is(ns, local))
    }

    /// 前序遍历（含自己），对应 ET `iter(tag)` 的文档顺序。
    pub(super) fn iter_tag<'a>(&'a self, ns: &str, local: &str) -> Vec<&'a Element> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(e) = stack.pop() {
            if e.is(ns, local) {
                out.push(e);
            }
            for c in e.children.iter().rev() {
                stack.push(c);
            }
        }
        out
    }
}

/// XML 1.0 允许的字符。
pub(super) fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}

fn is_xml_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// 读 `<?xml ... encoding="..."?>` 里声明的编码（小写）。
fn declared_encoding(bytes: &[u8]) -> Option<String> {
    if !bytes.starts_with(b"<?xml") {
        return None;
    }
    let head = bytes.get(..bytes.len().min(256))?;
    let end = head.windows(2).position(|w| w == b"?>")?;
    let decl = String::from_utf8_lossy(head.get(..end)?).to_ascii_lowercase();
    let after = decl.split("encoding").nth(1)?;
    let after = after.trim_start().strip_prefix('=')?.trim_start();
    let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let rest = after.get(1..)?;
    let value = rest.split(quote).next()?;
    Some(value.to_string())
}

/// 字节 -> 文本。UTF-8（含 BOM）、UTF-16LE/BE（含或不含 BOM）、声明为 ISO-8859-1 的文本。
pub(super) fn decode(bytes: &[u8]) -> Result<Cow<'_, str>, XmlError> {
    let utf16 = |body: &[u8], be: bool| -> Result<Cow<'_, str>, XmlError> {
        if !body.len().is_multiple_of(2) {
            return Err(XmlError::Malformed);
        }
        let units: Vec<u16> = body
            .chunks(2)
            .map(|p| {
                let (a, b) = (
                    p.first().copied().unwrap_or(0),
                    p.get(1).copied().unwrap_or(0),
                );
                if be {
                    u16::from_be_bytes([a, b])
                } else {
                    u16::from_le_bytes([a, b])
                }
            })
            .collect();
        String::from_utf16(&units)
            .map(Cow::Owned)
            .map_err(|_| XmlError::Malformed)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, false);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, true);
    }
    if bytes.starts_with(&[b'<', 0]) {
        return utf16(bytes, false);
    }
    if bytes.starts_with(&[0, b'<']) {
        return utf16(bytes, true);
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Some(enc) = declared_encoding(body) {
        if matches!(
            enc.as_str(),
            "iso-8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "l1"
        ) {
            return Ok(Cow::Owned(body.iter().map(|b| char::from(*b)).collect()));
        }
        if matches!(enc.as_str(), "us-ascii" | "ascii") && !body.is_ascii() {
            return Err(XmlError::Malformed);
        }
    }
    std::str::from_utf8(body)
        .map(Cow::Borrowed)
        .map_err(|_| XmlError::Malformed)
}

fn resolve_ns(res: ResolveResult<'_>) -> Result<Option<String>, XmlError> {
    match res {
        ResolveResult::Bound(ns) => Ok(Some(ns.0.to_string())),
        ResolveResult::Unbound => Ok(None),
        ResolveResult::Unknown(_) => Err(XmlError::Malformed),
    }
}

fn make_element(reader: &NsReader<&[u8]>, e: &BytesStart<'_>) -> Result<Element, XmlError> {
    let (res, local) = reader.resolver().resolve_element(e.name());
    let ns = resolve_ns(res)?;
    let mut attrs = Vec::new();
    for (count, a) in e.attributes().enumerate() {
        if count >= MAX_ATTRS_PER_ELEMENT {
            return Err(XmlError::TooComplex);
        }
        let a = a.map_err(|_| XmlError::Malformed)?;
        if a.key.as_namespace_binding().is_some() {
            continue;
        }
        let (ares, alocal) = reader.resolver().resolve_attribute(a.key);
        let ans = resolve_ns(ares)?;
        let value = a
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|_| XmlError::Malformed)?
            .into_owned();
        if !value.chars().all(is_xml_char) {
            return Err(XmlError::Malformed);
        }
        attrs.push(Attr {
            ns: ans,
            local: alocal.as_ref().to_string(),
            value,
        });
    }
    Ok(Element {
        ns,
        local: local.as_ref().to_string(),
        attrs,
        text: None,
        children: Vec::new(),
    })
}

fn attach(stack: &mut [Element], root: &mut Option<Element>, el: Element) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(el),
        None => *root = Some(el),
    }
}

fn push_text(stack: &mut [Element], s: &str) -> Result<(), XmlError> {
    if s.is_empty() {
        return Ok(());
    }
    if !s.chars().all(is_xml_char) {
        return Err(XmlError::Malformed);
    }
    match stack.last_mut() {
        Some(top) => {
            if top.children.is_empty() {
                top.text.get_or_insert_with(String::new).push_str(s);
            }
            Ok(())
        }
        None => {
            if s.chars().all(is_xml_ws) {
                Ok(())
            } else {
                Err(XmlError::Malformed)
            }
        }
    }
}

/// 解析一个 XML 部件（没有截止时间）。
#[cfg(test)]
pub(super) fn parse(bytes: &[u8]) -> Result<Element, XmlError> {
    parse_until(bytes, None)
}

/// 解析一个 XML 部件；`deadline` 到了就中止（协作式：每处理 1024 个事件检查一次时钟）。
pub(super) fn parse_until(
    bytes: &[u8],
    deadline: Option<std::time::Instant>,
) -> Result<Element, XmlError> {
    let text = decode(bytes)?;
    let mut reader = NsReader::from_str(&text);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    let mut nodes = 0usize;
    let mut events = 0u32;
    loop {
        events = events.wrapping_add(1);
        if events.is_multiple_of(1024) && deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            return Err(XmlError::Timeout);
        }
        let ev = reader.read_event().map_err(|_| XmlError::Malformed)?;
        match ev {
            Event::Start(e) => {
                if root.is_some() {
                    return Err(XmlError::Malformed);
                }
                nodes += 1;
                if nodes > MAX_NODES || stack.len() >= MAX_DEPTH {
                    return Err(XmlError::TooComplex);
                }
                stack.push(make_element(&reader, &e)?);
            }
            Event::Empty(e) => {
                if root.is_some() {
                    return Err(XmlError::Malformed);
                }
                nodes += 1;
                if nodes > MAX_NODES || stack.len() >= MAX_DEPTH {
                    return Err(XmlError::TooComplex);
                }
                let el = make_element(&reader, &e)?;
                attach(&mut stack, &mut root, el);
            }
            Event::End(_) => {
                let el = stack.pop().ok_or(XmlError::Malformed)?;
                attach(&mut stack, &mut root, el);
            }
            Event::Text(t) => push_text(&mut stack, &t.xml10_content())?,
            Event::CData(c) => {
                if stack.is_empty() {
                    return Err(XmlError::Malformed);
                }
                push_text(&mut stack, &c.xml10_content())?;
            }
            Event::GeneralRef(r) => {
                if stack.is_empty() {
                    return Err(XmlError::Malformed);
                }
                if r.is_char_ref() {
                    let ch = r
                        .resolve_char_ref()
                        .map_err(|_| XmlError::Malformed)?
                        .ok_or(XmlError::Malformed)?;
                    let mut buf = [0u8; 4];
                    push_text(&mut stack, ch.encode_utf8(&mut buf))?;
                } else {
                    let s = match &*r {
                        "lt" => "<",
                        "gt" => ">",
                        "amp" => "&",
                        "apos" => "'",
                        "quot" => "\"",
                        _ => return Err(XmlError::Malformed),
                    };
                    push_text(&mut stack, s)?;
                }
            }
            Event::DocType(_) => return Err(XmlError::Doctype),
            Event::Comment(_) | Event::PI(_) | Event::Decl(_) => {}
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err(XmlError::Malformed);
    }
    root.ok_or(XmlError::Malformed)
}

fn ascii_ci_contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle))
}

/// 这段字节像不像 XML：解码后（去掉 BOM / UTF-16 转码）第一个非空白字符是 `<`。
pub(super) fn looks_like_xml(bytes: &[u8]) -> bool {
    match decode(bytes) {
        Ok(t) => t
            .trim_start_matches(|c: char| is_xml_ws(c) || c == '\u{feff}')
            .starts_with('<'),
        Err(_) => false,
    }
}

/// 是否含有 DOCTYPE / ENTITY 声明（大小写不敏感；UTF-8 / UTF-16 都查；解码失败时退回按原始字节查）。
pub(super) fn contains_dtd(bytes: &[u8]) -> bool {
    let check = |b: &[u8]| ascii_ci_contains(b, b"<!doctype") || ascii_ci_contains(b, b"<!entity");
    if check(bytes) {
        return true;
    }
    match decode(bytes) {
        Ok(t) => check(t.as_bytes()),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

    fn p(s: &str) -> Result<Element, XmlError> {
        parse(s.as_bytes())
    }

    #[test]
    fn namespaces_by_uri_not_prefix() {
        let e = p(&format!(r#"<x:a xmlns:x="{M}"><x:b r="1"/></x:a>"#)).unwrap();
        assert!(e.is(M, "a"));
        assert!(e.find(M, "b").is_some());
        let e = p(&format!(r#"<a xmlns="{M}"><b/></a>"#)).unwrap();
        assert!(e.find(M, "b").is_some());
        assert!(p("<x:a/>").is_err());
    }

    #[test]
    fn text_semantics_match_elementtree() {
        let e = p("<a>x<!--c-->y<![CDATA[<z>]]>w<b/>tail</a>").unwrap();
        assert_eq!(e.text.as_deref(), Some("xy<z>w"));
        assert_eq!(p("<a></a>").unwrap().text, None);
        assert_eq!(p("<a/>").unwrap().text, None);
        assert_eq!(p("<a> </a>").unwrap().text.as_deref(), Some(" "));
        // 字面 CRLF / CR -> LF；字符引用保留 CR
        assert_eq!(
            p("<a>1\r\n2\r3&#13;4</a>").unwrap().text.as_deref(),
            Some("1\n2\n3\r4")
        );
        assert_eq!(
            p("<a>&lt;&amp;&gt;&quot;&apos;&#x41;&#66;</a>")
                .unwrap()
                .text
                .as_deref(),
            Some("<&>\"'AB")
        );
        // xml:space 被忽略
        assert_eq!(
            p(r#"<a xml:space="preserve"> x </a>"#)
                .unwrap()
                .text
                .as_deref(),
            Some(" x ")
        );
    }

    #[test]
    fn attribute_normalisation() {
        let e = p("<a k=\"1\n2\t3\r\n4&amp;&#10;\"/>").unwrap();
        assert_eq!(
            e.attr(None, "k"),
            Some(
                "1 2 3 4& \n"
                    .replace('\n', " ")
                    .as_str()
                    .get(..0)
                    .map(|_| "1 2 3 4&\n")
                    .unwrap_or("")
            )
        );
    }

    #[test]
    fn rejections() {
        assert_eq!(p("<!DOCTYPE a><a/>"), Err(XmlError::Doctype));
        assert_eq!(
            p("<!doctype a [<!ENTITY e \"x\">]><a>&e;</a>"),
            Err(XmlError::Doctype)
        );
        assert_eq!(p("<a>&e;</a>"), Err(XmlError::Malformed));
        assert_eq!(p("<a>"), Err(XmlError::Malformed));
        assert_eq!(p(""), Err(XmlError::Malformed));
        assert_eq!(p("<a/><b/>"), Err(XmlError::Malformed));
        assert_eq!(p("<a></b>"), Err(XmlError::Malformed));
        assert_eq!(p("<a>\u{1}</a>"), Err(XmlError::Malformed));
        assert_eq!(p("<a>&#1;</a>"), Err(XmlError::Malformed));
        assert_eq!(p("x<a/>"), Err(XmlError::Malformed));
        assert_eq!(parse(&[0xFF, 0xFE, 0x3C]), Err(XmlError::Malformed));
        assert_eq!(parse(b"<a>\xff</a>"), Err(XmlError::Malformed));
        let deep = format!("{}{}", "<a>".repeat(100), "</a>".repeat(100));
        assert_eq!(p(&deep), Err(XmlError::TooComplex));
    }

    #[test]
    fn utf16_and_bom() {
        let src = "<a>日本</a>";
        let mut le = vec![0xFF, 0xFE];
        le.extend(src.encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert_eq!(parse(&le).unwrap().text.as_deref(), Some("日本"));
        let be: Vec<u8> = src.encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(parse(&be).unwrap().text.as_deref(), Some("日本"));
        let mut u8bom = vec![0xEF, 0xBB, 0xBF];
        u8bom.extend(src.as_bytes());
        assert_eq!(parse(&u8bom).unwrap().text.as_deref(), Some("日本"));
        let dt: Vec<u8> = "<!DOCTYPE a><a/>"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(parse(&dt), Err(XmlError::Doctype));
        assert!(contains_dtd(&dt));
        assert!(contains_dtd(b"<!DocType x>"));
        assert!(!contains_dtd(b"<a>doctype</a>"));
        assert_eq!(
            parse(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>\xe9</a>")
                .unwrap()
                .text
                .as_deref(),
            Some("\u{e9}")
        );
    }

    #[test]
    fn iter_is_document_order_and_includes_root() {
        let e = p(&format!(
            r#"<w xmlns="{M}"><g><row r="1"/></g><row r="2"><row r="3"/></row></w>"#
        ))
        .unwrap();
        let rows: Vec<_> = e
            .iter_tag(M, "row")
            .iter()
            .map(|r| r.attr(None, "r").unwrap_or("").to_string())
            .collect();
        assert_eq!(rows, ["1", "2", "3"]);
    }
}
