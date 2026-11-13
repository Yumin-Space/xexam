//! DOCX (IX-3): `word/document.xml` with its numbering and styles. A paragraph is one block; tables give one block per row;
//! tracked deletions, field codes and everything in an `mc:Fallback` are skipped; the option letters and numbers Word
//! makes (list numbering) are written out in front of the text, because they are not in the text itself.
use super::xmlx::{walk, Ev, Tag, XmlBudget};
use super::zipx::ZipPkg;
use super::{Collector, Doc, ExtractError, FileKind, Limits};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
struct Level {
    start: u32,
    fmt: String,
    text: String,
}

#[derive(Debug, Default)]
struct Numbering {
    /// numId → abstractNumId
    nums: HashMap<String, String>,
    /// abstractNumId → levels by `ilvl`
    abstracts: HashMap<String, Vec<Level>>,
}

fn parse_numbering(
    bytes: &[u8],
    limits: &Limits,
    budget: &mut XmlBudget,
) -> Result<Numbering, ExtractError> {
    let mut n = Numbering::default();
    let mut cur_abs: Option<String> = None;
    let mut cur_lvl: Option<usize> = None;
    let mut cur_num: Option<String> = None;
    walk(bytes, limits, budget, |e| {
        match e {
            Ev::Start(t) => match t.name.as_str() {
                "abstractNum" => {
                    cur_abs = t.attr("abstractNumId").map(str::to_string);
                    if let Some(id) = &cur_abs {
                        n.abstracts.entry(id.clone()).or_default();
                    }
                }
                "lvl" => {
                    cur_lvl = t.attr("ilvl").and_then(|s| s.parse::<usize>().ok());
                    if let (Some(id), Some(l)) = (&cur_abs, cur_lvl) {
                        if l < 9 {
                            let v = n.abstracts.entry(id.clone()).or_default();
                            while v.len() <= l {
                                v.push(Level {
                                    start: 1,
                                    fmt: "decimal".into(),
                                    text: String::new(),
                                });
                            }
                        }
                    }
                }
                "start" | "numFmt" | "lvlText" => {
                    if let (Some(id), Some(l), Some(val)) = (&cur_abs, cur_lvl, t.attr("val")) {
                        if let Some(lv) = n.abstracts.get_mut(id).and_then(|v| v.get_mut(l)) {
                            match t.name.as_str() {
                                "start" => lv.start = val.parse().unwrap_or(1),
                                "numFmt" => lv.fmt = val.to_string(),
                                _ => lv.text = val.to_string(),
                            }
                        }
                    }
                }
                "num" => cur_num = t.attr("numId").map(str::to_string),
                "abstractNumId" if cur_num.is_some() && cur_abs.is_none() => {
                    if let (Some(num), Some(a)) = (&cur_num, t.attr("val")) {
                        n.nums.insert(num.clone(), a.to_string());
                    }
                }
                _ => {}
            },
            Ev::End(name) => match name {
                "abstractNum" => {
                    cur_abs = None;
                    cur_lvl = None;
                }
                "lvl" => cur_lvl = None,
                "num" => cur_num = None,
                _ => {}
            },
            Ev::Text(_) => {}
        }
        Ok(())
    })?;
    Ok(n)
}

/// style id → (basedOn, numId, ilvl)
type StyleMap = HashMap<String, (Option<String>, Option<String>, Option<usize>)>;

fn parse_styles(
    bytes: &[u8],
    limits: &Limits,
    budget: &mut XmlBudget,
) -> Result<StyleMap, ExtractError> {
    let mut map: StyleMap = HashMap::new();
    let mut cur: Option<String> = None;
    walk(bytes, limits, budget, |e| {
        match e {
            Ev::Start(t) => match t.name.as_str() {
                "style" => {
                    cur = t.attr("styleId").map(str::to_string);
                    if let Some(id) = &cur {
                        map.entry(id.clone()).or_default();
                    }
                }
                "basedOn" => {
                    if let (Some(id), Some(v)) = (&cur, t.attr("val")) {
                        if let Some(s) = map.get_mut(id) {
                            s.0 = Some(v.to_string());
                        }
                    }
                }
                "numId" => {
                    if let (Some(id), Some(v)) = (&cur, t.attr("val")) {
                        if let Some(s) = map.get_mut(id) {
                            s.1 = Some(v.to_string());
                        }
                    }
                }
                "ilvl" => {
                    if let (Some(id), Some(v)) = (&cur, t.attr("val")) {
                        if let Some(s) = map.get_mut(id) {
                            s.2 = v.parse().ok();
                        }
                    }
                }
                _ => {}
            },
            Ev::End("style") => cur = None,
            _ => {}
        }
        Ok(())
    })?;
    Ok(map)
}

/// The numbering of a style, following `basedOn` at most 10 steps.
fn style_num(styles: &StyleMap, style: &str) -> Option<(String, usize)> {
    let mut id = style.to_string();
    for _ in 0..10 {
        let s = styles.get(&id)?;
        if let Some(num) = &s.1 {
            return Some((num.clone(), s.2.unwrap_or(0)));
        }
        id = s.0.clone()?;
    }
    None
}

// —— number formats ——————————————————————————————————————————————————————————————————————————————————————————————————

fn letters(mut n: u32, upper: bool) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    out.reverse();
    let s: String = out.into_iter().collect();
    if upper {
        s.to_ascii_uppercase()
    } else {
        s
    }
}

fn roman(mut n: u32, upper: bool) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    if n == 0 || n >= 4000 {
        return n.to_string();
    }
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    if upper {
        s.to_ascii_uppercase()
    } else {
        s
    }
}

fn chinese(n: u32) -> String {
    const D: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    match n {
        0 => "零".into(),
        1..=9 => D[n as usize].into(),
        10 => "十".into(),
        11..=19 => format!("十{}", D[(n % 10) as usize]),
        20..=99 => {
            let t = D[(n / 10) as usize];
            if n.is_multiple_of(10) {
                format!("{t}十")
            } else {
                format!("{t}十{}", D[(n % 10) as usize])
            }
        }
        _ => n.to_string(),
    }
}

const AIUEO: &str =
    "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン";
const IROHA: &str = "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス";

fn nth_char(s: &str, n: u32) -> Option<char> {
    s.chars().nth(n.checked_sub(1)? as usize)
}

fn format_counter(fmt: &str, n: u32) -> String {
    match fmt {
        "decimalZero" => format!("{n:02}"),
        "upperLetter" => letters(n, true),
        "lowerLetter" => letters(n, false),
        "upperRoman" => roman(n, true),
        "lowerRoman" => roman(n, false),
        "decimalEnclosedCircle" => match n {
            1..=20 => {
                char::from_u32(0x2460 + n - 1).map_or_else(|| n.to_string(), |c| c.to_string())
            }
            _ => n.to_string(),
        },
        "chineseCounting" | "chineseCountingThousand" | "ideographTraditional" => chinese(n),
        "aiueoFullWidth" => nth_char(AIUEO, n).map_or_else(|| n.to_string(), |c| c.to_string()),
        "iroha" => nth_char(IROHA, n).map_or_else(|| n.to_string(), |c| c.to_string()),
        _ => n.to_string(),
    }
}

/// Per-`numId` counters by level.
#[derive(Default)]
struct Counters {
    by_num: HashMap<String, [Option<u32>; 9]>,
}

impl Counters {
    /// Advances the counter of `ilvl` (deeper levels reset) and writes the prefix of the level.
    fn prefix(&mut self, num: &str, ilvl: usize, numbering: &Numbering) -> String {
        if num == "0" || ilvl > 8 {
            return String::new();
        }
        let levels = numbering
            .nums
            .get(num)
            .and_then(|a| numbering.abstracts.get(a));
        let level_of = |l: usize| -> Level {
            levels.and_then(|v| v.get(l)).cloned().unwrap_or(Level {
                start: 1,
                fmt: "decimal".into(),
                text: String::new(),
            })
        };
        let c = self.by_num.entry(num.to_string()).or_default();
        let lv = level_of(ilvl);
        c[ilvl] = Some(match c[ilvl] {
            Some(n) => n + 1,
            None => lv.start,
        });
        for d in c.iter_mut().skip(ilvl + 1) {
            *d = None;
        }
        if lv.fmt == "none" {
            return String::new();
        }
        if lv.fmt == "bullet" {
            return "•".to_string();
        }
        let template = if lv.text.is_empty() {
            format!("%{}.", ilvl + 1)
        } else {
            lv.text.clone()
        };
        let mut out = String::new();
        let mut chars = template.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '%' {
                if let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                    chars.next();
                    let l = (d as usize).saturating_sub(1).min(8);
                    let n = c[l].unwrap_or_else(|| level_of(l).start);
                    out.push_str(&format_counter(&level_of(l).fmt, n));
                    continue;
                }
            }
            out.push(ch);
        }
        out
    }
}

// —— document ————————————————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Default)]
struct Para {
    text: String,
    style: Option<String>,
    num: Option<(String, usize)>,
    in_ppr: bool,
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<String>>,
    cell: Vec<String>,
    in_row: bool,
}

struct State<'a> {
    numbering: &'a Numbering,
    styles: &'a StyleMap,
    counters: Counters,
    paras: Vec<Para>,
    tables: Vec<Table>,
    /// > 0 while inside a deleted run / field code / skipped alternative
    skip: usize,
    /// text goes only into `w:t` elements
    in_t: bool,
    /// per `mc:AlternateContent`: the first `mc:Choice` has been read
    choice_seen: Vec<bool>,
    /// per `mc:Choice` / `mc:Fallback` start: whether it is skipped (so End knows)
    alt_stack: Vec<bool>,
    out: Vec<String>,
}

impl State<'_> {
    fn start(&mut self, t: &Tag) {
        match t.name.as_str() {
            "AlternateContent" => self.choice_seen.push(false),
            "Choice" => {
                let skipped = match self.choice_seen.last_mut() {
                    Some(seen) if *seen => true,
                    Some(seen) => {
                        *seen = true;
                        false
                    }
                    None => false,
                };
                self.alt_stack.push(skipped);
                if skipped {
                    self.skip += 1;
                }
            }
            "Fallback" => {
                self.alt_stack.push(true);
                self.skip += 1;
            }
            "del" | "delText" | "instrText" | "footnoteReference" | "commentReference" => {
                self.skip += 1;
            }
            "tbl" => self.tables.push(Table::default()),
            "tr" => {
                if self.tables.len() == 1 {
                    let tb = &mut self.tables[0];
                    tb.in_row = true;
                    tb.rows.push(Vec::new());
                }
            }
            "tc" => {
                if self.tables.len() == 1 {
                    self.tables[0].cell.clear();
                }
            }
            "p" => self.paras.push(Para::default()),
            "pPr" => {
                if let Some(p) = self.paras.last_mut() {
                    p.in_ppr = true;
                }
            }
            "pStyle" => {
                if let (Some(p), Some(v)) = (self.paras.last_mut(), t.attr("val")) {
                    if p.in_ppr {
                        p.style = Some(v.to_string());
                    }
                }
            }
            "numId" => {
                if let (Some(p), Some(v)) = (self.paras.last_mut(), t.attr("val")) {
                    if p.in_ppr && self.skip == 0 {
                        let lvl = p.num.as_ref().map_or(0, |(_, l)| *l);
                        p.num = Some((v.to_string(), lvl));
                    }
                }
            }
            "ilvl" => {
                if let (Some(p), Some(v)) = (self.paras.last_mut(), t.attr("val")) {
                    if p.in_ppr {
                        let id = p.num.as_ref().map_or(String::new(), |(i, _)| i.clone());
                        p.num = Some((id, v.parse().unwrap_or(0)));
                    }
                }
            }
            "t" => self.in_t = true,
            "tab" if self.skip == 0 => self.push_text("\t"),
            "br" | "cr" if self.skip == 0 => self.push_text("\n"),
            "noBreakHyphen" if self.skip == 0 => self.push_text("-"),
            _ => {}
        }
    }

    fn push_text(&mut self, s: &str) {
        if let Some(p) = self.paras.last_mut() {
            p.text.push_str(s);
        }
    }

    fn end(&mut self, name: &str) {
        match name {
            "AlternateContent" => {
                self.choice_seen.pop();
            }
            "Choice" | "Fallback" => {
                if self.alt_stack.pop() == Some(true) {
                    self.skip = self.skip.saturating_sub(1);
                }
            }
            "del" | "delText" | "instrText" | "footnoteReference" | "commentReference" => {
                self.skip = self.skip.saturating_sub(1);
            }
            "t" => self.in_t = false,
            "pPr" => {
                if let Some(p) = self.paras.last_mut() {
                    p.in_ppr = false;
                }
            }
            "p" => self.end_paragraph(),
            "tc" => {
                if self.tables.len() == 1 {
                    let tb = &mut self.tables[0];
                    let cell = tb.cell.join(" / ");
                    if let Some(row) = tb.rows.last_mut() {
                        row.push(cell);
                    }
                    tb.cell.clear();
                }
            }
            "tr" => {
                if self.tables.len() == 1 {
                    let tb = &mut self.tables[0];
                    tb.in_row = false;
                    if let Some(row) = tb.rows.pop() {
                        let cells: Vec<String> = row
                            .into_iter()
                            .map(|c| c.trim().to_string())
                            .filter(|c| !c.is_empty())
                            .collect();
                        if !cells.is_empty() {
                            self.out.push(cells.join(" | "));
                        }
                    }
                }
            }
            "tbl" => {
                self.tables.pop();
            }
            _ => {}
        }
    }

    fn end_paragraph(&mut self) {
        let Some(p) = self.paras.pop() else {
            return;
        };
        // the number of a list paragraph (own numPr, else its style's)
        let num = p
            .num
            .clone()
            .filter(|(id, _)| !id.is_empty())
            .or_else(|| p.style.as_deref().and_then(|s| style_num(self.styles, s)));
        let prefix = match &num {
            Some((id, lvl)) => self.counters.prefix(id, *lvl, self.numbering),
            None => String::new(),
        };
        let text = p.text.trim();
        if text.is_empty() {
            return;
        }
        let line = if prefix.is_empty() {
            text.to_string()
        } else {
            format!("{prefix} {text}")
        };
        // inside a table (any depth) the paragraph belongs to the outermost cell
        if let Some(tb) = self.tables.first_mut() {
            tb.cell.push(line);
        } else {
            self.out.push(line);
        }
    }
}

pub fn extract(bytes: &[u8], limits: &Limits) -> Result<Doc, ExtractError> {
    let mut pkg = ZipPkg::open(bytes, limits)?;
    let mut budget = XmlBudget::default();
    let numbering = match pkg.read("word/numbering.xml")? {
        Some(b) => parse_numbering(&b, limits, &mut budget)?,
        None => Numbering::default(),
    };
    let styles = match pkg.read("word/styles.xml")? {
        Some(b) => parse_styles(&b, limits, &mut budget)?,
        None => StyleMap::new(),
    };
    let doc = pkg
        .read("word/document.xml")?
        .ok_or(ExtractError::Corrupt)?;
    let mut st = State {
        numbering: &numbering,
        styles: &styles,
        counters: Counters::default(),
        paras: Vec::new(),
        tables: Vec::new(),
        skip: 0,
        in_t: false,
        choice_seen: Vec::new(),
        alt_stack: Vec::new(),
        out: Vec::new(),
    };
    let mut col = Collector::new(limits.max_chars);
    col.begin("");
    walk(&doc, limits, &mut budget, |e| {
        match e {
            Ev::Start(t) => st.start(t),
            Ev::End(n) => st.end(n),
            Ev::Text(s) => {
                if st.in_t && st.skip == 0 {
                    st.push_text(s);
                }
            }
        }
        for block in st.out.drain(..) {
            col.add(&block)?;
        }
        Ok(())
    })?;
    Ok(col.finish(FileKind::Docx, 0, None))
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::extract::testgen::{DocxSpec, Para as P};

    fn read(spec: &DocxSpec) -> Vec<String> {
        let z = crate::extract::testgen::docx(spec);
        let l = Limits {
            min_meaningful: 0,
            ..Limits::default()
        };
        extract(&z, &l).unwrap().sections[0].blocks.clone()
    }

    #[test]
    fn text_tabs_and_breaks() {
        let mut s = DocxSpec::default();
        s.body.push(P::runs(
            "<w:r><w:t>Which tool</w:t><w:tab/><w:t>keeps</w:t><w:br/><w:t>a pot?</w:t></w:r>",
        ));
        let b = read(&s);
        assert_eq!(b, vec!["Which tool\tkeeps\na pot?"]);
    }

    #[test]
    fn tracked_deletions_and_field_codes_are_skipped_insertions_are_read() {
        let mut s = DocxSpec::default();
        s.body.push(P::runs(
            "<w:r><w:t>kept </w:t></w:r><w:del><w:r><w:delText>gone</w:delText><w:t>gone2</w:t></w:r></w:del>\
             <w:ins><w:r><w:t>added </w:t></w:r></w:ins>\
             <w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:t>end</w:t></w:r>",
        ));
        assert_eq!(read(&s), vec!["kept added end"]);
    }

    #[test]
    fn a_text_box_is_read_once() {
        let mut s = DocxSpec::default();
        s.body.push(P::runs(
            "<w:r><mc:AlternateContent><mc:Choice Requires=\"wps\"><w:drawing><wps:txbx><w:txbxContent>\
             <w:p><w:r><w:t>boxed text</w:t></w:r></w:p></w:txbxContent></wps:txbx></w:drawing></mc:Choice>\
             <mc:Fallback><w:pict><w:txbxContent><w:p><w:r><w:t>boxed text</w:t></w:r></w:p></w:txbxContent></w:pict></mc:Fallback>\
             </mc:AlternateContent></w:r>",
        ));
        s.body.push(P::text("after the box"));
        let b = read(&s);
        assert_eq!(b, vec!["boxed text", "after the box"]);
    }

    #[test]
    fn tables_give_one_block_per_row_with_slashes_between_paragraphs_of_a_cell() {
        let mut s = DocxSpec::default();
        s.body.push(P::raw(
            "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>Q1</w:t></w:r></w:p></w:tc>\
             <w:tc><w:p><w:r><w:t>first</w:t></w:r></w:p><w:p><w:r><w:t>second</w:t></w:r></w:p></w:tc></w:tr>\
             <w:tr><w:tc><w:p><w:r><w:t>Q2</w:t></w:r></w:p></w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>",
        ));
        assert_eq!(read(&s), vec!["Q1 | first / second", "Q2"]);
    }

    #[test]
    fn a_nested_table_is_flattened_into_its_cell() {
        let mut s = DocxSpec::default();
        s.body.push(P::raw(
            "<w:tbl><w:tr><w:tc><w:p><w:r><w:t>outer</w:t></w:r></w:p>\
             <w:tbl><w:tr><w:tc><w:p><w:r><w:t>inner a</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>inner b</w:t></w:r></w:p></w:tc></w:tr></w:tbl>\
             </w:tc></w:tr></w:tbl>",
        ));
        assert_eq!(read(&s), vec!["outer / inner a / inner b"]);
    }

    #[test]
    fn decimal_and_letter_lists() {
        let mut s = DocxSpec::default();
        s.numbering = Some(DocxSpec::numbering_xml(&[
            ("1", "0", "decimal", "%1.", 1),
            ("1", "1", "upperLetter", "%2.", 1),
        ]));
        s.body.push(P::numbered("Question one", "1", 0));
        s.body.push(P::numbered("first option", "1", 1));
        s.body.push(P::numbered("second option", "1", 1));
        s.body.push(P::numbered("Question two", "1", 0));
        s.body.push(P::numbered("again first", "1", 1));
        assert_eq!(
            read(&s),
            vec![
                "1. Question one",
                "A. first option",
                "B. second option",
                "2. Question two",
                "A. again first"
            ]
        );
    }

    #[test]
    fn a_list_style_from_styles_xml_and_start_values() {
        let mut s = DocxSpec::default();
        s.numbering = Some(DocxSpec::numbering_xml(&[(
            "5",
            "0",
            "lowerRoman",
            "(%1)",
            3,
        )]));
        s.styles = Some(
            "<w:styles xmlns:w=\"u\"><w:style w:styleId=\"Base\"><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"5\"/></w:numPr></w:pPr></w:style>\
             <w:style w:styleId=\"Child\"><w:basedOn w:val=\"Base\"/></w:style></w:styles>"
                .to_string(),
        );
        s.body.push(P::styled("styled item", "Child"));
        s.body.push(P::styled("next item", "Child"));
        assert_eq!(read(&s), vec!["(iii) styled item", "(iv) next item"]);
    }

    #[test]
    fn east_asian_formats_and_bullets() {
        let mut s = DocxSpec::default();
        s.numbering = Some(DocxSpec::numbering_xml(&[
            ("1", "0", "aiueoFullWidth", "%1)", 1),
            ("2", "0", "chineseCounting", "%1、", 1),
            ("3", "0", "bullet", "o", 1),
            ("4", "0", "decimalEnclosedCircle", "%1", 1),
        ]));
        s.body.push(P::numbered("kana one", "1", 0));
        s.body.push(P::numbered("kana two", "1", 0));
        s.body.push(P::numbered("中文一", "2", 0));
        s.body.push(P::numbered("中文二", "2", 0));
        s.body.push(P::numbered("dot", "3", 0));
        s.body.push(P::numbered("circled", "4", 0));
        assert_eq!(
            read(&s),
            vec![
                "ア) kana one",
                "イ) kana two",
                "一、 中文一",
                "二、 中文二",
                "• dot",
                "① circled"
            ]
        );
    }

    #[test]
    fn a_missing_main_part_is_corrupt_and_a_doctype_is_refused() {
        let z = crate::extract::testgen::zip_of(&[("word/other.xml", b"<a/>")]);
        assert_eq!(extract(&z, &Limits::default()), Err(ExtractError::Corrupt));
        let bad = crate::extract::testgen::zip_of(&[(
            "word/document.xml",
            &crate::extract::testgen::xml_doctype_entities()[..],
        )]);
        assert_eq!(
            extract(&bad, &Limits::default()),
            Err(ExtractError::TooComplex)
        );
    }

    #[test]
    fn the_character_cap_stops_the_extraction() {
        let mut s = DocxSpec::default();
        for i in 0..400 {
            s.body.push(P::text(&format!(
                "paragraph number {i} with some filler words to add length"
            )));
        }
        let z = crate::extract::testgen::docx(&s);
        assert_eq!(
            extract(&z, &Limits::small()),
            Err(ExtractError::TooMuchText)
        );
    }
}
