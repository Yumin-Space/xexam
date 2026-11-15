//! v1.6 Word export: one bank as a `.docx` for reading and printing. The package is written by hand (zip + WordprocessingML
//! string building, no extra dependency, no fonts inside). Reading order: title, then English, Chinese and Japanese, each
//! on its own page and holding every sheet with every question, its answer and explanation. Headings use Word's built-in
//! "heading 1" / "heading 2" styles (navigation pane). Runs carry `w:lang` (and font hints only), so the viewer picks the
//! right CJK glyph shapes from its own fonts. A missing Chinese or Japanese text falls back to the English text.
use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::{xml_text, ExportBank, ExportError};
use crate::models::{Answer, ExplainText, Kind, Question, Zh};

/// The Word document of `bank` (see the v1.6 contract §4): bank title, then English, Chinese and Japanese, each with every
/// sheet and every question with its answer and explanation. `now` goes into docProps/core.xml.
pub fn write_docx(bank: &ExportBank, now: jiff::Timestamp) -> Result<Vec<u8>, ExportError> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let parts: [(&str, String); 9] = [
        ("[Content_Types].xml", CONTENT_TYPES.to_string()),
        ("_rels/.rels", ROOT_RELS.to_string()),
        ("docProps/core.xml", core_xml(&bank.name, now)),
        ("docProps/app.xml", APP_XML.to_string()),
        ("word/document.xml", document_xml(bank)),
        ("word/styles.xml", STYLES_XML.to_string()),
        ("word/settings.xml", SETTINGS_XML.to_string()),
        ("word/fontTable.xml", FONT_TABLE_XML.to_string()),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS.to_string()),
    ];
    for (name, body) in &parts {
        zip.start_file(*name, opts)?;
        zip.write_all(body.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

// ---------------------------------------------------------------------------------------------------------------------
// Static parts

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

const CONTENT_TYPES: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
    "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
    "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    "<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>",
    "<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>",
    "<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>",
    "<Override PartName=\"/word/fontTable.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml\"/>",
    "<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>",
    "<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>",
    "</Types>"
);

const ROOT_RELS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>",
    "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>",
    "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>",
    "</Relationships>"
);

const DOCUMENT_RELS: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
    "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>",
    "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable\" Target=\"fontTable.xml\"/>",
    "</Relationships>"
);

const APP_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">",
    "<Application>Xexam</Application>",
    "</Properties>"
);

const W_NS: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

const STYLES_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
    "<w:docDefaults>",
    "<w:rPrDefault><w:rPr>",
    "<w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Arial\" w:eastAsia=\"Microsoft YaHei\"/>",
    "<w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/>",
    "<w:lang w:val=\"en-US\" w:eastAsia=\"zh-CN\"/>",
    "</w:rPr></w:rPrDefault>",
    "<w:pPrDefault><w:pPr><w:spacing w:after=\"80\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>",
    "</w:docDefaults>",
    "<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\">",
    "<w:name w:val=\"Normal\"/><w:uiPriority w:val=\"0\"/><w:qFormat/>",
    "</w:style>",
    "<w:style w:type=\"paragraph\" w:styleId=\"Title\">",
    "<w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"10\"/><w:qFormat/>",
    "<w:pPr><w:spacing w:after=\"240\"/></w:pPr>",
    "<w:rPr><w:b/><w:bCs/><w:sz w:val=\"48\"/><w:szCs w:val=\"48\"/></w:rPr>",
    "</w:style>",
    "<w:style w:type=\"paragraph\" w:styleId=\"Heading1\">",
    "<w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/>",
    "<w:pPr><w:keepNext/><w:spacing w:before=\"360\" w:after=\"120\"/><w:outlineLvl w:val=\"0\"/></w:pPr>",
    "<w:rPr><w:b/><w:bCs/><w:color w:val=\"1F3864\"/><w:sz w:val=\"32\"/><w:szCs w:val=\"32\"/></w:rPr>",
    "</w:style>",
    "<w:style w:type=\"paragraph\" w:styleId=\"Heading2\">",
    "<w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/>",
    "<w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"80\"/><w:outlineLvl w:val=\"1\"/></w:pPr>",
    "<w:rPr><w:b/><w:bCs/><w:color w:val=\"2F5496\"/><w:sz w:val=\"26\"/><w:szCs w:val=\"26\"/></w:rPr>",
    "</w:style>",
    "</w:styles>"
);

// Schema order of CT_Settings: defaultTabStop, characterSpacingControl, compat, themeFontLang.
const SETTINGS_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
    "<w:defaultTabStop w:val=\"720\"/>",
    "<w:characterSpacingControl w:val=\"doNotCompress\"/>",
    "<w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat>",
    "<w:themeFontLang w:val=\"en-US\" w:eastAsia=\"zh-CN\"/>",
    "</w:settings>"
);

// Font names are hints only; nothing is embedded and the viewer's own fonts render the text.
const FONT_TABLE_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
    "<w:fonts xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
    "<w:font w:name=\"Calibri\"><w:altName w:val=\"Arial\"/></w:font>",
    "<w:font w:name=\"Arial\"/>",
    "<w:font w:name=\"Microsoft YaHei\"><w:altName w:val=\"PingFang SC\"/></w:font>",
    "<w:font w:name=\"Yu Gothic\"><w:altName w:val=\"Hiragino Sans\"/></w:font>",
    "</w:fonts>"
);

fn core_xml(title: &str, now: jiff::Timestamp) -> String {
    let when = now.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    format!(
        "{XML_DECL}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <dc:title>{}</dc:title><dc:creator>Xexam</dc:creator><cp:lastModifiedBy>Xexam</cp:lastModifiedBy>\
         <dcterms:created xsi:type=\"dcterms:W3CDTF\">{when}</dcterms:created>\
         <dcterms:modified xsi:type=\"dcterms:W3CDTF\">{when}</dcterms:modified></cp:coreProperties>",
        xml_text(title)
    )
}

// ---------------------------------------------------------------------------------------------------------------------
// Paragraph and run building

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    En,
    Zh,
    Ja,
}

impl Lang {
    fn rfonts(self) -> &'static str {
        match self {
            Lang::En => "",
            Lang::Zh => "<w:rFonts w:eastAsia=\"Microsoft YaHei\" w:hint=\"eastAsia\"/>",
            Lang::Ja => "<w:rFonts w:eastAsia=\"Yu Gothic\" w:hint=\"eastAsia\"/>",
        }
    }

    fn lang_tag(self) -> &'static str {
        match self {
            Lang::En => "<w:lang w:val=\"en-US\"/>",
            Lang::Zh => "<w:lang w:val=\"zh-CN\" w:eastAsia=\"zh-CN\"/>",
            Lang::Ja => "<w:lang w:val=\"ja-JP\" w:eastAsia=\"ja-JP\"/>",
        }
    }
}

/// A piece of text with the language it is written in.
#[derive(Clone, Debug)]
struct Txt {
    s: String,
    lang: Lang,
}

impl Txt {
    fn new(s: impl Into<String>, lang: Lang) -> Txt {
        Txt { s: s.into(), lang }
    }
}

#[derive(Debug)]
struct Run {
    text: String,
    lang: Lang,
    bold: bool,
    color: Option<&'static str>,
    /// Half-points
    size: Option<u32>,
}

impl Run {
    fn plain(t: &Txt) -> Run {
        Run {
            text: t.s.clone(),
            lang: t.lang,
            bold: false,
            color: None,
            size: None,
        }
    }

    fn bold(text: impl Into<String>, lang: Lang) -> Run {
        Run {
            text: text.into(),
            lang,
            bold: true,
            color: None,
            size: None,
        }
    }

    fn check_mark() -> Run {
        Run {
            text: " \u{2713}".to_string(),
            lang: Lang::En,
            bold: true,
            color: Some("1E7B34"),
            size: None,
        }
    }

    fn xml(&self) -> String {
        let mut rpr = String::from(self.lang.rfonts());
        if self.bold {
            rpr.push_str("<w:b/><w:bCs/>");
        }
        if let Some(c) = self.color {
            rpr.push_str(&format!("<w:color w:val=\"{c}\"/>"));
        }
        if let Some(sz) = self.size {
            rpr.push_str(&format!("<w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/>"));
        }
        rpr.push_str(self.lang.lang_tag());
        format!(
            "<w:r><w:rPr>{rpr}</w:rPr>{}</w:r>",
            text_content(&self.text)
        )
    }
}

/// The content of a run: a TAB becomes `<w:tab/>`, a line break `<w:br/>`; every text carries `xml:space="preserve"`.
fn text_content(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::new();
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push_str("<w:br/>");
        }
        for (j, piece) in line.split('\t').enumerate() {
            if j > 0 {
                out.push_str("<w:tab/>");
            }
            if !piece.is_empty() {
                out.push_str(&format!(
                    "<w:t xml:space=\"preserve\">{}</w:t>",
                    xml_text(piece)
                ));
            }
        }
    }
    out
}

#[derive(Default, Debug)]
struct Para {
    style: Option<&'static str>,
    keep_next: bool,
    page_break_before: bool,
    /// Twips of space after
    after: Option<u32>,
    /// Twips of left indent
    left: Option<u32>,
    runs: Vec<Run>,
}

impl Para {
    fn xml(&self) -> String {
        let mut ppr = String::new();
        if let Some(s) = self.style {
            ppr.push_str(&format!("<w:pStyle w:val=\"{s}\"/>"));
        }
        if self.keep_next {
            ppr.push_str("<w:keepNext/>");
        }
        if self.page_break_before {
            ppr.push_str("<w:pageBreakBefore/>");
        }
        if let Some(a) = self.after {
            ppr.push_str(&format!("<w:spacing w:after=\"{a}\"/>"));
        }
        if let Some(l) = self.left {
            ppr.push_str(&format!("<w:ind w:left=\"{l}\"/>"));
        }
        let ppr = if ppr.is_empty() {
            String::new()
        } else {
            format!("<w:pPr>{ppr}</w:pPr>")
        };
        let runs: String = self.runs.iter().map(Run::xml).collect();
        format!("<w:p>{ppr}{runs}</w:p>")
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Language texts

struct Labels {
    answer: &'static str,
    explanation: &'static str,
    memory: &'static str,
    note: &'static str,
    none: &'static str,
}

impl Lang {
    fn labels(self) -> Labels {
        match self {
            Lang::En => Labels {
                answer: "Answer: ",
                explanation: "Explanation: ",
                memory: "Memory aid: ",
                note: "Note: ",
                none: "(no questions)",
            },
            Lang::Zh => Labels {
                answer: "答案：",
                explanation: "解析：",
                memory: "记忆口诀：",
                note: "备注：",
                none: "（没有题目）",
            },
            Lang::Ja => Labels {
                answer: "正解：",
                explanation: "解説：",
                memory: "覚え方：",
                note: "備考：",
                none: "（問題がありません）",
            },
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Zh => "中文",
            Lang::Ja => "日本語",
        }
    }

    fn kind_tag(self, kind: Kind) -> &'static str {
        match (self, kind) {
            (Lang::En, Kind::Single) => "[Single choice]",
            (Lang::En, Kind::Multi) => "[Multiple choice]",
            (Lang::En, Kind::Hotspot) => "[Hotspot]",
            (Lang::Zh, Kind::Single) => "［单选题］",
            (Lang::Zh, Kind::Multi) => "［多选题］",
            (Lang::Zh, Kind::Hotspot) => "［热点题］",
            (Lang::Ja, Kind::Single) => "［単一選択］",
            (Lang::Ja, Kind::Multi) => "［複数選択］",
            (Lang::Ja, Kind::Hotspot) => "［ホットスポット］",
        }
    }
}

/// The translation struct of the section language, if any.
fn translation(q: &Question, lang: Lang) -> Option<&Zh> {
    match lang {
        Lang::En => None,
        Lang::Zh => q.zh.as_ref(),
        Lang::Ja => q.ja.as_ref(),
    }
}

/// `own` when it is present and non-empty (then in `lang`), else the English text.
fn pick(own: Option<&str>, english: &str, lang: Lang) -> Txt {
    match own {
        Some(s) if !s.is_empty() => Txt::new(s, lang),
        _ => Txt::new(english, Lang::En),
    }
}

fn stem_of(q: &Question, lang: Lang) -> Txt {
    pick(translation(q, lang).map(|t| t.stem.as_str()), &q.stem, lang)
}

fn option_of(q: &Question, lang: Lang, k: usize) -> Txt {
    let own = translation(q, lang)
        .and_then(|t| t.options.as_ref())
        .and_then(|o| o.get(k))
        .map(String::as_str);
    pick(own, q.options.get(k).map_or("", |o| o.text.as_str()), lang)
}

fn item_of(q: &Question, lang: Lang, i: usize) -> Txt {
    let own = translation(q, lang)
        .and_then(|t| t.items.as_ref())
        .and_then(|o| o.get(i))
        .map(String::as_str);
    pick(own, q.items.get(i).map_or("", |o| o.text.as_str()), lang)
}

/// The choices of hotspot item `i`: the section language's when present with the English count, else English.
fn choices_of(q: &Question, lang: Lang, i: usize) -> Vec<Txt> {
    let english: &[String] = q.items.get(i).map_or(&[], |it| it.choices.as_slice());
    let own = translation(q, lang)
        .and_then(|t| t.choices.as_ref())
        .and_then(|c| c.get(i));
    match own {
        Some(c) if c.len() == english.len() => {
            c.iter().map(|s| Txt::new(s.as_str(), lang)).collect()
        }
        _ => english
            .iter()
            .map(|s| Txt::new(s.as_str(), Lang::En))
            .collect(),
    }
}

/// Explanation, memory aid and note of the section language, with the language the text is written in.
fn explain_of(q: &Question, lang: Lang) -> (ExplainText, Lang) {
    let non_empty = |t: &Option<ExplainText>| t.as_ref().filter(|t| !t.is_empty()).cloned();
    match &q.explain {
        Some(e) => {
            let own = match lang {
                Lang::En => None,
                Lang::Zh => non_empty(&e.zh),
                Lang::Ja => non_empty(&e.ja),
            };
            match own {
                Some(t) => (t, lang),
                None => (e.en.clone().unwrap_or_default(), Lang::En),
            }
        }
        None => (
            ExplainText {
                explanation: q.explanation.clone(),
                mnemonic: q.mnemonic.clone(),
                note: q.note.clone(),
            },
            lang,
        ),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Document body

/// Title language: Japanese when it has kana, Chinese when it has ideographs, else English.
fn title_lang(s: &str) -> Lang {
    if s.chars().any(|c| ('\u{3040}'..='\u{30FF}').contains(&c)) {
        Lang::Ja
    } else if s.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c)) {
        Lang::Zh
    } else {
        Lang::En
    }
}

fn label_value(label: &str, lang: Lang, value: Vec<Run>) -> Para {
    let mut runs = vec![Run::bold(label, lang)];
    runs.extend(value);
    Para {
        runs,
        ..Para::default()
    }
}

fn is_correct_letter(q: &Question, key: &str) -> bool {
    matches!(&q.answer, Answer::Letters(l) if l.iter().any(|a| a.trim().eq_ignore_ascii_case(key.trim())))
}

fn question_paras(q: &Question, n: usize, lang: Lang) -> Vec<Para> {
    let labels = lang.labels();
    let mut out: Vec<Para> = Vec::new();

    // 1. Number line
    out.push(Para {
        keep_next: true,
        runs: vec![
            Run::bold(format!("{n}. "), Lang::En),
            Run {
                text: lang.kind_tag(q.kind).to_string(),
                lang,
                bold: false,
                color: Some("666666"),
                size: Some(18),
            },
        ],
        ..Para::default()
    });

    // 2. Stem
    out.push(Para {
        runs: vec![Run::plain(&stem_of(q, lang))],
        ..Para::default()
    });

    let hotspot = q.kind == Kind::Hotspot;
    if hotspot {
        // 4. Items with their choices
        let idx: &[i64] = match &q.answer {
            Answer::Indexes(v) => v,
            Answer::Letters(_) => &[],
        };
        for i in 0..q.items.len() {
            let item = item_of(q, lang, i);
            out.push(Para {
                left: Some(360),
                keep_next: true,
                runs: vec![Run::plain(&Txt::new(
                    format!("{}. {}", i + 1, item.s),
                    item.lang,
                ))],
                ..Para::default()
            });
            let chosen = idx.get(i).and_then(|v| usize::try_from(*v).ok());
            let mut runs = Vec::new();
            for (j, c) in choices_of(q, lang, i).iter().enumerate() {
                if j > 0 {
                    runs.push(Run::plain(&Txt::new(" / ", Lang::En)));
                }
                if Some(j) == chosen {
                    runs.push(Run::bold(c.s.clone(), c.lang));
                    runs.push(Run::check_mark());
                } else {
                    runs.push(Run::plain(c));
                }
            }
            out.push(Para {
                left: Some(720),
                runs,
                ..Para::default()
            });
        }
    } else {
        // 3. Options
        for (k, o) in q.options.iter().enumerate() {
            let t = option_of(q, lang, k);
            let text = Txt::new(format!("{}. {}", o.key, t.s), t.lang);
            let runs = if is_correct_letter(q, &o.key) {
                vec![Run::bold(text.s, text.lang), Run::check_mark()]
            } else {
                vec![Run::plain(&text)]
            };
            out.push(Para {
                left: Some(360),
                runs,
                ..Para::default()
            });
        }
    }

    // 5. Answer line
    let value = if hotspot {
        match &q.answer {
            Answer::Indexes(idx) => {
                let parts: Vec<Txt> = idx
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let choice = usize::try_from(*v)
                            .ok()
                            .and_then(|j| choices_of(q, lang, i).into_iter().nth(j));
                        match choice {
                            Some(c) => Txt::new(format!("{}. {}", i + 1, c.s), c.lang),
                            None => Txt::new(format!("{}. ?", i + 1), Lang::En),
                        }
                    })
                    .collect();
                join_txt(&parts, "; ")
            }
            Answer::Letters(_) => vec![Run::plain(&Txt::new("?", Lang::En))],
        }
    } else {
        match &q.answer {
            Answer::Letters(l) => vec![Run::plain(&Txt::new(l.join(", "), Lang::En))],
            Answer::Indexes(_) => vec![Run::plain(&Txt::new("?", Lang::En))],
        }
    };
    out.push(label_value(labels.answer, lang, value));

    // 6-8. Explanation, memory aid, note
    let (ex, ex_lang) = explain_of(q, lang);
    for (label, text) in [
        (labels.explanation, &ex.explanation),
        (labels.memory, &ex.mnemonic),
        (labels.note, &ex.note),
    ] {
        if !text.is_empty() {
            out.push(label_value(
                label,
                lang,
                vec![Run::plain(&Txt::new(text.as_str(), ex_lang))],
            ));
        }
    }

    // 9. Space after the question
    if let Some(last) = out.last_mut() {
        last.after = Some(240);
    }
    out
}

/// Runs for texts joined by a separator (each text keeps its own language).
fn join_txt(parts: &[Txt], sep: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            runs.push(Run::plain(&Txt::new(sep, Lang::En)));
        }
        runs.push(Run::plain(p));
    }
    runs
}

fn heading(style: &'static str, text: &str, lang: Lang, page_break: bool) -> Para {
    Para {
        style: Some(style),
        page_break_before: page_break,
        runs: vec![Run::plain(&Txt::new(text, lang))],
        ..Para::default()
    }
}

fn document_xml(bank: &ExportBank) -> String {
    let mut body = String::new();
    body.push_str(
        &Para {
            style: Some("Title"),
            runs: vec![Run::plain(&Txt::new(
                bank.name.as_str(),
                title_lang(&bank.name),
            ))],
            ..Para::default()
        }
        .xml(),
    );
    for (li, lang) in [Lang::En, Lang::Zh, Lang::Ja].into_iter().enumerate() {
        body.push_str(&heading("Heading1", lang.heading(), lang, li > 0).xml());
        if bank.sheets.is_empty() {
            body.push_str(
                &Para {
                    runs: vec![Run::plain(&Txt::new(lang.labels().none, lang))],
                    ..Para::default()
                }
                .xml(),
            );
        }
        for sheet in &bank.sheets {
            body.push_str(&heading("Heading2", &sheet.name, lang, false).xml());
            for (qi, q) in sheet.questions.iter().enumerate() {
                for p in question_paras(q, qi + 1, lang) {
                    body.push_str(&p.xml());
                }
            }
        }
    }
    body.push_str(
        "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
         <w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\" w:header=\"567\" w:footer=\"567\" w:gutter=\"0\"/>\
         </w:sectPr>",
    );
    format!(
        "{XML_DECL}<w:document xmlns:w=\"{W_NS}\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><w:body>{body}</w:body></w:document>"
    )
}
