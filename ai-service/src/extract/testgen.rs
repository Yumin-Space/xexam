//! Generators of made-up test files (Office packages, PDFs, bombs, encodings). Always compiled so the integration tests
//! of ai-service can build fixtures; the release binary drops it (nothing calls it). Every text is invented (a fictional
//! kitchen-equipment course); no real question text, no secrets.
use std::io::Write;

// —— zip ————————————————————————————————————————————————————————————————————————————————————————————————————————————

fn opts() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)
}

/// A zip with the given members (deflated).
pub fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in members {
        w.start_file(*name, opts()).expect("start");
        w.write_all(data).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

/// A zip with one member `bomb.bin` of `mb` MiB of zeros (a few KB compressed).
pub fn zip_bomb_member(mb: usize) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    w.start_file("bomb.bin", opts()).expect("start");
    let chunk = vec![0u8; 1 << 20];
    for _ in 0..mb {
        w.write_all(&chunk).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

/// A zip with `n` empty members.
pub fn zip_many_entries(n: usize) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..n {
        w.start_file(format!("e{i}"), o).expect("start");
    }
    w.finish().expect("finish").into_inner()
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// A zip whose member `lie.bin` really holds 200 000 zero bytes but whose headers declare 100 (local header and
/// central directory): the declared size is a lie that a reader must not trust.
pub fn zip_lying_size() -> Vec<u8> {
    let mut z = zip_of(&[("lie.bin", &vec![0u8; 200_000][..])]);
    let local = find(&z, &[b'P', b'K', 3, 4], 0).expect("local header");
    let central = find(&z, &[b'P', b'K', 1, 2], 0).expect("central header");
    let hundred = 100u32.to_le_bytes();
    z[local + 18..local + 22].copy_from_slice(&hundred);
    z[local + 22..local + 26].copy_from_slice(&hundred);
    z[central + 20..central + 24].copy_from_slice(&hundred);
    z[central + 24..central + 28].copy_from_slice(&hundred);
    z
}

/// A zip with two members of the same name (both headers patched to the same bytes).
pub fn zip_with_duplicate_name() -> Vec<u8> {
    let mut z = zip_of(&[("aa", b"one"), ("ab", b"two")]);
    let mut at = 0;
    while let Some(p) = find(&z, b"ab", at) {
        z[p + 1] = b'a';
        at = p + 2;
    }
    z
}

/// A zip whose end record says `classic` entries and whose zip64 record says `zip64` (the central directory itself is
/// the honest one-member directory of `zip_of`).
pub fn zip64_claiming(classic: u16, zip64: u64) -> Vec<u8> {
    let z = zip_of(&[("a", b"x")]);
    let eocd = find(&z, &[b'P', b'K', 5, 6], 0).expect("eocd");
    let cd_size = u32::from_le_bytes([z[eocd + 12], z[eocd + 13], z[eocd + 14], z[eocd + 15]]);
    let cd_off = u32::from_le_bytes([z[eocd + 16], z[eocd + 17], z[eocd + 18], z[eocd + 19]]);
    let mut out = z[..eocd].to_vec();
    let rec_at = out.len() as u64;
    out.extend_from_slice(&[b'P', b'K', 6, 6]);
    out.extend_from_slice(&44u64.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&45u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&zip64.to_le_bytes());
    out.extend_from_slice(&zip64.to_le_bytes());
    out.extend_from_slice(&u64::from(cd_size).to_le_bytes());
    out.extend_from_slice(&u64::from(cd_off).to_le_bytes());
    out.extend_from_slice(&[b'P', b'K', 6, 7]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&rec_at.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&[b'P', b'K', 5, 6]);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&classic.to_le_bytes());
    out.extend_from_slice(&classic.to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

// —— XML ————————————————————————————————————————————————————————————————————————————————————————————————————————————

/// A billion-laughs style document (a DOCTYPE with nested entities).
pub fn xml_doctype_entities() -> Vec<u8> {
    let mut s = String::from("<?xml version=\"1.0\"?>\n<!DOCTYPE lolz [\n<!ENTITY lol \"lol\">\n");
    for i in 1..=9 {
        let prev = if i == 1 {
            "lol".to_string()
        } else {
            format!("lol{}", i - 1)
        };
        let body = format!("&{prev};").repeat(10);
        s.push_str(&format!("<!ENTITY lol{i} \"{body}\">\n"));
    }
    s.push_str("]>\n<lolz>&lol9;</lolz>");
    s.into_bytes()
}

/// `depth` nested `<a>` elements.
pub fn xml_deep(depth: usize) -> Vec<u8> {
    format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth)).into_bytes()
}

// —— text encodings ——————————————————————————————————————————————————————————————————————————————————————————————————

pub fn utf8_bom(s: &str) -> Vec<u8> {
    let mut v = vec![0xEF, 0xBB, 0xBF];
    v.extend_from_slice(s.as_bytes());
    v
}

pub fn utf16le_bom(s: &str) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v
}

pub fn utf16be_bom(s: &str) -> Vec<u8> {
    let mut v = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        v.extend_from_slice(&u.to_be_bytes());
    }
    v
}

pub fn gb18030(s: &str) -> Vec<u8> {
    encoding_rs::GB18030.encode(s).0.into_owned()
}

pub fn shift_jis(s: &str) -> Vec<u8> {
    encoding_rs::SHIFT_JIS.encode(s).0.into_owned()
}

/// A CSV with quoted commas, doubled quotes and a newline inside a field.
pub fn csv_tricky() -> String {
    "id,question,note\r\n1,\"Which tool, if any, keeps a \"\"steady\"\" simmer?\",ok\r\n2,Second row,\"line one\nline two\"\r\n"
        .to_string()
}

// —— DOCX ———————————————————————————————————————————————————————————————————————————————————————————————————————————

/// One body element of a generated DOCX.
#[derive(Debug, Clone)]
pub enum Para {
    /// inserted as it is (a table, for example)
    Raw(String),
    /// the content of one `w:p` (runs)
    Runs(String),
    Text(String),
    Numbered(String, String, usize),
    Styled(String, String),
}

impl Para {
    pub fn raw(xml: &str) -> Para {
        Para::Raw(xml.to_string())
    }
    pub fn runs(xml: &str) -> Para {
        Para::Runs(xml.to_string())
    }
    pub fn text(t: &str) -> Para {
        Para::Text(t.to_string())
    }
    pub fn numbered(t: &str, num_id: &str, ilvl: usize) -> Para {
        Para::Numbered(t.to_string(), num_id.to_string(), ilvl)
    }
    pub fn styled(t: &str, style: &str) -> Para {
        Para::Styled(t.to_string(), style.to_string())
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The pieces of a generated DOCX.
#[derive(Debug, Clone, Default)]
pub struct DocxSpec {
    pub body: Vec<Para>,
    pub numbering: Option<String>,
    pub styles: Option<String>,
}

impl DocxSpec {
    pub fn paragraphs(texts: &[&str]) -> DocxSpec {
        DocxSpec {
            body: texts.iter().map(|t| Para::text(t)).collect(),
            ..DocxSpec::default()
        }
    }

    /// A `numbering.xml` with one abstract numbering (same id) per distinct `numId`; rows are (numId, ilvl, numFmt,
    /// lvlText, start).
    pub fn numbering_xml(rows: &[(&str, &str, &str, &str, u32)]) -> String {
        let mut ids: Vec<&str> = Vec::new();
        for r in rows {
            if !ids.contains(&r.0) {
                ids.push(r.0);
            }
        }
        let mut s = String::from("<w:numbering xmlns:w=\"urn:w\">");
        for id in &ids {
            s.push_str(&format!("<w:abstractNum w:abstractNumId=\"{id}\">"));
            for r in rows.iter().filter(|r| r.0 == *id) {
                s.push_str(&format!(
                    "<w:lvl w:ilvl=\"{}\"><w:start w:val=\"{}\"/><w:numFmt w:val=\"{}\"/><w:lvlText w:val=\"{}\"/></w:lvl>",
                    r.1, r.4, r.2, esc(r.3)
                ));
            }
            s.push_str("</w:abstractNum>");
        }
        for id in &ids {
            s.push_str(&format!(
                "<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"{id}\"/></w:num>"
            ));
        }
        s.push_str("</w:numbering>");
        s
    }
}

pub fn docx(spec: &DocxSpec) -> Vec<u8> {
    let mut body = String::new();
    for p in &spec.body {
        match p {
            Para::Raw(x) => body.push_str(x),
            Para::Runs(x) => body.push_str(&format!("<w:p>{x}</w:p>")),
            Para::Text(t) => body.push_str(&format!("<w:p><w:r><w:t>{}</w:t></w:r></w:p>", esc(t))),
            Para::Numbered(t, id, lvl) => body.push_str(&format!(
                "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"{lvl}\"/><w:numId w:val=\"{id}\"/></w:numPr></w:pPr><w:r><w:t>{}</w:t></w:r></w:p>",
                esc(t)
            )),
            Para::Styled(t, style) => body.push_str(&format!(
                "<w:p><w:pPr><w:pStyle w:val=\"{style}\"/></w:pPr><w:r><w:t>{}</w:t></w:r></w:p>",
                esc(t)
            )),
        }
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"urn:w\" xmlns:mc=\"urn:mc\" xmlns:wps=\"urn:wps\"><w:body>{body}</w:body></w:document>"
    );
    let mut members: Vec<(&str, Vec<u8>)> = vec![
        ("[Content_Types].xml", b"<Types/>".to_vec()),
        ("word/document.xml", document.into_bytes()),
    ];
    if let Some(n) = &spec.numbering {
        members.push(("word/numbering.xml", n.clone().into_bytes()));
    }
    if let Some(s) = &spec.styles {
        members.push(("word/styles.xml", s.clone().into_bytes()));
    }
    let refs: Vec<(&str, &[u8])> = members.iter().map(|(n, d)| (*n, d.as_slice())).collect();
    zip_of(&refs)
}

// —— XLSX ———————————————————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Debug, Clone)]
pub struct SheetSpec {
    pub name: String,
    pub rows_xml: String,
    pub hidden: bool,
}

impl SheetSpec {
    pub fn new(name: &str, rows_xml: &str, hidden: bool) -> SheetSpec {
        SheetSpec {
            name: name.to_string(),
            rows_xml: rows_xml.to_string(),
            hidden,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct XlsxSpec {
    /// raw `<si>` elements
    pub shared: Vec<String>,
    pub sheets: Vec<SheetSpec>,
    /// more `<sheet>` elements that point at the first sheet's part
    pub alias_sheets: Vec<String>,
    /// raw `<Relationship>` elements added to the workbook relationships
    pub extra_rels: Vec<String>,
    /// more `<sheet>` elements (name, r:id)
    pub extra_sheet_refs: Vec<(String, String)>,
}

impl XlsxSpec {
    /// Three sheets (one hidden) with shared, inline, number, boolean, string and error cells.
    pub fn simple() -> XlsxSpec {
        XlsxSpec {
            shared: vec![
                "<si><t>Which tool keeps a pot at a steady simmer?</t></si>".into(),
                "<si><t>Thermostat</t></si>".into(),
            ],
            sheets: vec![
                SheetSpec::new(
                    "Ovens",
                    "<row><c t=\"s\"><v>0</v></c><c t=\"s\"><v>1</v></c><c><v>42</v></c><c t=\"b\"><v>1</v></c><c t=\"inlineStr\"><is><t>inline note</t></is></c><c t=\"e\"><v>#DIV/0!</v></c></row>",
                    false,
                ),
                SheetSpec::new(
                    "Knives",
                    "<row><c t=\"str\"><v>Second sheet row</v></c><c t=\"b\"><v>0</v></c></row>",
                    false,
                ),
                SheetSpec::new(
                    "Hidden",
                    "<row><c t=\"inlineStr\"><is><t>secret row</t></is></c></row>",
                    true,
                ),
            ],
            ..XlsxSpec::default()
        }
    }
}

pub fn xlsx(spec: &XlsxSpec) -> Vec<u8> {
    let mut sheets_xml = String::new();
    let mut rels = String::from("<Relationships>");
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, s) in spec.sheets.iter().enumerate() {
        let n = i + 1;
        sheets_xml.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{n}\"{} r:id=\"rId{n}\"/>",
            esc(&s.name),
            if s.hidden { " state=\"hidden\"" } else { "" }
        ));
        rels.push_str(&format!(
            "<Relationship Id=\"rId{n}\" Type=\"urn:t/worksheet\" Target=\"worksheets/sheet{n}.xml\"/>"
        ));
        parts.push((
            format!("xl/worksheets/sheet{n}.xml"),
            format!(
                "<worksheet><sheetData>{}</sheetData></worksheet>",
                s.rows_xml
            )
            .into_bytes(),
        ));
    }
    for (k, name) in spec.alias_sheets.iter().enumerate() {
        sheets_xml.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId1\"/>",
            esc(name),
            100 + k
        ));
    }
    for (name, rid) in &spec.extra_sheet_refs {
        sheets_xml.push_str(&format!(
            "<sheet name=\"{}\" sheetId=\"200\" r:id=\"{rid}\"/>",
            esc(name)
        ));
    }
    rels.push_str(
        "<Relationship Id=\"rIdSS\" Type=\"urn:t/sharedStrings\" Target=\"sharedStrings.xml\"/>",
    );
    for r in &spec.extra_rels {
        rels.push_str(r);
    }
    rels.push_str("</Relationships>");
    let workbook = format!("<workbook xmlns:r=\"urn:r\"><sheets>{sheets_xml}</sheets></workbook>");
    let shared = format!("<sst>{}</sst>", spec.shared.concat());
    let mut members: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), b"<Types/>".to_vec()),
        ("xl/workbook.xml".into(), workbook.into_bytes()),
        ("xl/_rels/workbook.xml.rels".into(), rels.into_bytes()),
        ("xl/sharedStrings.xml".into(), shared.into_bytes()),
    ];
    members.extend(parts);
    let refs: Vec<(&str, &[u8])> = members
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    zip_of(&refs)
}

// —— PPTX ———————————————————————————————————————————————————————————————————————————————————————————————————————————

#[derive(Debug, Clone, Default)]
pub struct PptxSpec {
    /// the text of each slide part, in file order
    pub slides: Vec<String>,
    /// the order of the slide list (indexes into `slides`); empty = file order
    pub order: Vec<usize>,
    /// the first listed slide gets a table and a notes page
    pub extras: bool,
}

impl PptxSpec {
    pub fn simple() -> PptxSpec {
        PptxSpec {
            slides: vec!["Second slide body".into(), "First slide title".into()],
            order: vec![1, 0],
            extras: true,
        }
    }
}

pub fn pptx(spec: &PptxSpec) -> Vec<u8> {
    let order: Vec<usize> = if spec.order.is_empty() {
        (0..spec.slides.len()).collect()
    } else {
        spec.order.clone()
    };
    let first = order.first().copied();
    let mut list = String::new();
    let mut rels = String::from("<Relationships>");
    for (k, &i) in order.iter().enumerate() {
        list.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + k,
            i + 1
        ));
    }
    for i in 0..spec.slides.len() {
        rels.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"urn:t/slide\" Target=\"slides/slide{}.xml\"/>",
            i + 1,
            i + 1
        ));
    }
    rels.push_str("</Relationships>");
    let presentation = format!(
        "<p:presentation xmlns:r=\"urn:r\"><p:sldIdLst>{list}</p:sldIdLst></p:presentation>"
    );
    let mut members: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), b"<Types/>".to_vec()),
        ("ppt/presentation.xml".into(), presentation.into_bytes()),
        ("ppt/_rels/presentation.xml.rels".into(), rels.into_bytes()),
    ];
    for (i, text) in spec.slides.iter().enumerate() {
        let extras = spec.extras && first == Some(i);
        let mut xml = format!(
            "<p:sld><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{}</a:t></a:r></a:p></p:txBody></p:sp>",
            esc(text)
        );
        if extras {
            xml.push_str(
                "<p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr>\
                 <a:tc><a:txBody><a:p><a:r><a:t>Row A</a:t></a:r></a:p></a:txBody></a:tc>\
                 <a:tc><a:txBody><a:p><a:r><a:t>Row B</a:t></a:r></a:p></a:txBody></a:tc>\
                 </a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>",
            );
        }
        xml.push_str("</p:spTree></p:cSld></p:sld>");
        members.push((format!("ppt/slides/slide{}.xml", i + 1), xml.into_bytes()));
        if extras {
            members.push((
                format!("ppt/slides/_rels/slide{}.xml.rels", i + 1),
                b"<Relationships><Relationship Id=\"rId1\" Type=\"urn:t/notesSlide\" Target=\"../notesSlides/notesSlide1.xml\"/></Relationships>".to_vec(),
            ));
            members.push((
                "ppt/notesSlides/notesSlide1.xml".into(),
                b"<p:notes><p:cSld><p:spTree>\
                  <p:sp><p:nvSpPr><p:nvPr><p:ph type=\"sldImg\"/></p:nvPr></p:nvSpPr></p:sp>\
                  <p:sp><p:nvSpPr><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:r><a:t>Remember the simmer question</a:t></a:r></a:p></p:txBody></p:sp>\
                  <p:sp><p:nvSpPr><p:nvPr><p:ph type=\"sldNum\"/></p:nvPr></p:nvSpPr><p:txBody><a:p><a:fld><a:t>slide number 7</a:t></a:fld></a:p></p:txBody></p:sp>\
                  </p:spTree></p:cSld></p:notes>"
                    .to_vec(),
            ));
        }
    }
    let refs: Vec<(&str, &[u8])> = members
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    zip_of(&refs)
}

// —— PDF ————————————————————————————————————————————————————————————————————————————————————————————————————————————

/// A minimal PDF writer: numbered objects, a classic xref table, a trailer.
struct PdfBuilder {
    objects: Vec<Vec<u8>>,
}

impl PdfBuilder {
    fn new() -> PdfBuilder {
        PdfBuilder {
            objects: Vec::new(),
        }
    }

    /// Adds an object body and returns its number (1-based).
    fn add(&mut self, body: &[u8]) -> usize {
        self.objects.push(body.to_vec());
        self.objects.len()
    }

    fn stream(&mut self, dict_extra: &str, data: &[u8]) -> usize {
        let mut b = format!("<< /Length {} {dict_extra} >>\nstream\n", data.len()).into_bytes();
        b.extend_from_slice(data);
        b.extend_from_slice(b"\nendstream");
        self.add(&b)
    }

    fn finish(self, root: usize, trailer_extra: &str) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(o);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", self.objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for off in &offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

fn pdf_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Pages of Helvetica text (WinAnsi); `""` makes an empty page.
pub fn pdf_latin(pages: &[&str]) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let catalog = b.add(b"<< /Type /Catalog /Pages 2 0 R >>");
    let pages_id = b.add(b""); // patched below
    debug_assert_eq!(pages_id, 2);
    let font =
        b.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    let mut kids = Vec::new();
    for text in pages {
        let content = if text.is_empty() {
            Vec::new()
        } else {
            format!("BT /F1 12 Tf 72 720 Td ({}) Tj ET", pdf_escape(text)).into_bytes()
        };
        let c = b.stream("", &content);
        let p = b.add(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
            )
            .as_bytes(),
        );
        kids.push(format!("{p} 0 R"));
    }
    b.objects[1] = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    )
    .into_bytes();
    b.finish(catalog, "")
}

/// `n` pages without text.
pub fn pdf_empty(n: usize) -> Vec<u8> {
    let texts = vec![""; n];
    pdf_latin(&texts)
}

/// `n` pages that share one content stream.
pub fn pdf_pages(n: usize) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let catalog = b.add(b"<< /Type /Catalog /Pages 2 0 R >>");
    b.add(b"");
    let font = b.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    let c = b.stream(
        "",
        b"BT /F1 12 Tf 72 720 Td (shared page text for the page count test) Tj ET",
    );
    let mut kids = Vec::new();
    for _ in 0..n {
        let p = b.add(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> /Contents {c} 0 R >>"
            )
            .as_bytes(),
        );
        kids.push(format!("{p} 0 R"));
    }
    b.objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).into_bytes();
    b.finish(catalog, "")
}

/// One page of CJK text in a Type0 / Identity-H font without an embedded font; a ToUnicode map gives the characters.
pub fn pdf_cjk(text: &str) -> Vec<u8> {
    let mut chars: Vec<char> = Vec::new();
    for c in text.chars() {
        if !chars.contains(&c) {
            chars.push(c);
        }
    }
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for chunk in chars.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for c in chunk {
            let code = chars.iter().position(|x| x == c).unwrap_or(0) + 1;
            let mut u16s = [0u16; 2];
            let enc = c.encode_utf16(&mut u16s);
            let hex: String = enc.iter().map(|u| format!("{u:04X}")).collect();
            cmap.push_str(&format!("<{code:04X}> <{hex}>\n"));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend");
    let shown: String = text
        .chars()
        .map(|c| {
            let code = chars.iter().position(|x| *x == c).unwrap_or(0) + 1;
            format!("{code:04X}")
        })
        .collect();
    let mut b = PdfBuilder::new();
    let catalog = b.add(b"<< /Type /Catalog /Pages 2 0 R >>");
    b.add(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 7 0 R >>");
    b.add(b"<< /Type /Font /Subtype /Type0 /BaseFont /TestCJK /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 6 0 R >>");
    b.add(b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /TestCJK /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /DW 1000 /FontDescriptor 8 0 R >>");
    b.stream("", cmap.as_bytes());
    b.stream(
        "",
        format!("BT /F1 12 Tf 72 720 Td <{shown}> Tj ET").as_bytes(),
    );
    b.add(b"<< /Type /FontDescriptor /FontName /TestCJK /Flags 4 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>");
    b.finish(catalog, "")
}

/// A PDF with an `/Encrypt` dictionary that the empty password cannot open.
pub fn pdf_encrypted_junk() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let catalog = b.add(b"<< /Type /Catalog /Pages 2 0 R >>");
    b.add(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>");
    b.stream("", b"BT ET");
    let o = "AB".repeat(32);
    let u = "CD".repeat(32);
    let enc =
        b.add(format!("<< /Filter /Standard /V 1 /R 2 /O <{o}> /U <{u}> /P -4 >>").as_bytes());
    b.finish(
        catalog,
        &format!("/Encrypt {enc} 0 R /ID [<00112233445566778899AABBCCDDEEFF> <00112233445566778899AABBCCDDEEFF>]"),
    )
}

/// A font whose `/ToUnicode` is a Name instead of a stream (some libraries panic on it).
pub fn pdf_bad_tounicode() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let catalog = b.add(b"<< /Type /Catalog /Pages 2 0 R >>");
    b.add(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>");
    b.add(b"<< /Type /Font /Subtype /Type0 /BaseFont /Bad /Encoding /Identity-H /ToUnicode /Identity-H /DescendantFonts [6 0 R] >>");
    b.stream("", b"BT /F1 12 Tf 72 720 Td <00010002> Tj ET");
    b.add(b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Bad /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> >>");
    b.finish(catalog, "")
}

/// A page whose Flate content stream inflates to `mb` MiB (zeros).
pub fn pdf_flate_bomb(mb: usize) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::best());
    let chunk = vec![b' '; 1 << 20];
    for _ in 0..mb {
        enc.write_all(&chunk).expect("deflate");
    }
    let data = enc.finish().expect("finish");
    let mut b = PdfBuilder::new();
    let catalog = b.add(b"<< /Type /Catalog /Pages 2 0 R >>");
    b.add(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>");
    b.stream("/Filter /FlateDecode", &data);
    b.finish(catalog, "")
}

/// A page with an object of `n` nested arrays (a recursion bomb for a parser).
pub fn pdf_nested_arrays(n: usize) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let catalog = b.add(b"<< /Type /Catalog /Pages 2 0 R >>");
    b.add(b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Junk 5 0 R >>");
    b.stream("", b"BT ET");
    b.add(format!("{}{}", "[".repeat(n), "]".repeat(n)).as_bytes());
    b.finish(catalog, "")
}
