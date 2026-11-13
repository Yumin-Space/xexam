//! Zip packages (OOXML) with hard limits (IX-3, SEC-2, SEC-3). Nothing is ever written from a zip (no paths, no zip-slip
//! surface); nested zips are never opened; a member is read only through `take(limit + 1)` so a declared size is never
//! trusted or used to allocate; the central directory is checked by hand before the library sees it.
use super::{ExtractError, Limits};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};

/// An opened package.
pub struct ZipPkg<'a> {
    archive: zip::ZipArchive<Cursor<&'a [u8]>>,
    index: HashMap<String, usize>,
    total_read: u64,
    member_cap: u64,
    total_cap: u64,
    read_names: HashSet<String>,
}

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;

fn u16_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 2)
        .map(|s| u64::from(u16::from_le_bytes([s[0], s[1]])))
}
fn u32_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 4)
        .map(|s| u64::from(u32::from_le_bytes([s[0], s[1], s[2], s[3]])))
}
fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| {
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        u64::from_le_bytes(a)
    })
}

/// SEC-2: parses the end-of-central-directory record (and the zip64 one) before the library does. Declared entries above
/// the cap, a central directory that reaches outside the input, or a zip64 count that disagrees with the classic count
/// when both are set are refused; nothing is allocated from a declared size or count.
pub fn precheck(bytes: &[u8], limits: &Limits) -> Result<(), ExtractError> {
    let len = bytes.len();
    if len < 22 {
        return Err(ExtractError::Corrupt);
    }
    // the record sits in the last 22 + 65 535 bytes
    let lowest = len.saturating_sub(22 + 65_535);
    let mut at = len - 22;
    let eocd = loop {
        if u32_at(bytes, at) == Some(u64::from(EOCD_SIG)) {
            break at;
        }
        if at == lowest {
            return Err(ExtractError::Corrupt);
        }
        at -= 1;
    };
    let entries = u16_at(bytes, eocd + 10).ok_or(ExtractError::Corrupt)?;
    let cd_size = u32_at(bytes, eocd + 12).ok_or(ExtractError::Corrupt)?;
    let cd_off = u32_at(bytes, eocd + 16).ok_or(ExtractError::Corrupt)?;
    let (mut entries_eff, mut size_eff, mut off_eff) = (entries, cd_size, cd_off);
    let zip64_needed = entries == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF;
    if eocd >= 20 && u32_at(bytes, eocd - 20) == Some(u64::from(ZIP64_LOCATOR_SIG)) {
        let rec_at = u64_at(bytes, eocd - 20 + 8).ok_or(ExtractError::Corrupt)?;
        let rec_at = usize::try_from(rec_at).map_err(|_| ExtractError::Corrupt)?;
        if u32_at(bytes, rec_at) != Some(u64::from(ZIP64_EOCD_SIG)) {
            return Err(ExtractError::Corrupt);
        }
        let n64 = u64_at(bytes, rec_at + 32).ok_or(ExtractError::Corrupt)?;
        let size64 = u64_at(bytes, rec_at + 40).ok_or(ExtractError::Corrupt)?;
        let off64 = u64_at(bytes, rec_at + 48).ok_or(ExtractError::Corrupt)?;
        if entries != 0xFFFF && entries != n64 {
            return Err(ExtractError::Corrupt);
        }
        entries_eff = n64;
        size_eff = size64;
        off_eff = off64;
    } else if zip64_needed {
        return Err(ExtractError::Corrupt);
    }
    if entries_eff > limits.zip_entries as u64 {
        return Err(ExtractError::TooComplex);
    }
    let end = off_eff.checked_add(size_eff).ok_or(ExtractError::Corrupt)?;
    if end > eocd as u64 {
        return Err(ExtractError::Corrupt);
    }
    // Walk the central directory ourselves: duplicate member names are refused (the library would merge them). The
    // walk is bounded by the entry cap and by the directory's own bytes, which lie inside the input.
    let mut names: HashSet<&[u8]> = HashSet::new();
    let mut at = usize::try_from(off_eff).map_err(|_| ExtractError::Corrupt)?;
    for _ in 0..entries_eff {
        if u32_at(bytes, at) != Some(0x0201_4b50) {
            return Err(ExtractError::Corrupt);
        }
        let name_len = u16_at(bytes, at + 28).ok_or(ExtractError::Corrupt)? as usize;
        let extra_len = u16_at(bytes, at + 30).ok_or(ExtractError::Corrupt)? as usize;
        let comment_len = u16_at(bytes, at + 32).ok_or(ExtractError::Corrupt)? as usize;
        let name = bytes
            .get(at + 46..at + 46 + name_len)
            .ok_or(ExtractError::Corrupt)?;
        if !names.insert(name) {
            return Err(ExtractError::Corrupt);
        }
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(())
}

impl<'a> ZipPkg<'a> {
    /// Opens the package: pre-check, library parse, entry cap, duplicate names → `Corrupt`.
    pub fn open(bytes: &'a [u8], limits: &Limits) -> Result<ZipPkg<'a>, ExtractError> {
        precheck(bytes, limits)?;
        let archive =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| ExtractError::Corrupt)?;
        if archive.len() > limits.zip_entries {
            return Err(ExtractError::TooComplex);
        }
        let mut index = HashMap::new();
        let mut archive = archive;
        for i in 0..archive.len() {
            let name = archive
                .by_index_raw(i)
                .map_err(|_| ExtractError::Corrupt)?
                .name()
                .map(|n| n.into_owned())
                .map_err(|_| ExtractError::Corrupt)?;
            if index.insert(name, i).is_some() {
                return Err(ExtractError::Corrupt);
            }
        }
        Ok(ZipPkg {
            archive,
            index,
            total_read: 0,
            member_cap: limits.zip_member,
            total_cap: limits.zip_total,
            read_names: HashSet::new(),
        })
    }

    /// `true` when the package has a member with exactly this name.
    pub fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// The member's bytes, `None` when it does not exist. At most `zip_member` bytes per member and `zip_total` for all
    /// members read (`TooComplex`); an encrypted member is `Protected`; a read or CRC error is `Corrupt`.
    pub fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, ExtractError> {
        let Some(&i) = self.index.get(name) else {
            return Ok(None);
        };
        if self
            .archive
            .by_index_raw(i)
            .map_err(|_| ExtractError::Corrupt)?
            .encrypted()
        {
            return Err(ExtractError::Protected);
        }
        let file = match self.archive.by_index(i) {
            Ok(f) => f,
            Err(zip::result::ZipError::UnsupportedArchive(m))
                if m.to_ascii_lowercase().contains("password") =>
            {
                return Err(ExtractError::Protected)
            }
            Err(_) => return Err(ExtractError::Corrupt),
        };
        let room = self.member_cap.min(self.total_cap - self.total_read);
        let mut out = Vec::new();
        let mut limited = file.take(room + 1);
        limited
            .read_to_end(&mut out)
            .map_err(|_| ExtractError::Corrupt)?;
        if out.len() as u64 > room {
            return Err(ExtractError::TooComplex);
        }
        self.total_read += out.len() as u64;
        Ok(Some(out))
    }

    /// Like [`ZipPkg::read`], but a part is read at most once per extraction: a second request for the same name gives
    /// `None` (SEC-3: 200 sheets pointing at one part read it once).
    pub fn read_once(&mut self, name: &str) -> Result<Option<Vec<u8>>, ExtractError> {
        if !self.read_names.insert(name.to_string()) {
            return Ok(None);
        }
        self.read(name)
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let h = (b[i + 1] as char).to_digit(16);
            let l = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// SEC-3: resolves a relationship target inside the package. `from` is the part that holds the relationship (its
/// directory is the base of a relative target). The target is percent-decoded; a `/`-rooted one starts at the package
/// root; `.` and `..` are normalised and never go above the root (`None` then); `external` targets give `None` (nothing is
/// ever fetched).
pub fn resolve_target(from: &str, target: &str, external: bool) -> Option<String> {
    if external {
        return None;
    }
    let target = percent_decode(target);
    // a URL scheme means a link, not a part
    if let Some((scheme, _)) = target.split_once(':') {
        if !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        {
            return None;
        }
    }
    let target = target.split('#').next().unwrap_or("").replace('\\', "/");
    if !target.split('/').any(|s| !s.is_empty() && s != ".") {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    if !target.starts_with('/') {
        let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        parts.extend(dir.split('/').filter(|p| !p.is_empty()));
    }
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

/// The relationships part of `part` (`dir/_rels/file.rels`).
pub fn rels_name(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// One relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rel {
    pub id: String,
    pub kind: String,
    /// The resolved package path (`None` for an external or unresolvable target)
    pub target: Option<String>,
}

/// Reads the relationships of `part`.
pub fn read_rels(
    pkg: &mut ZipPkg<'_>,
    part: &str,
    limits: &Limits,
    budget: &mut super::xmlx::XmlBudget,
) -> Result<Vec<Rel>, ExtractError> {
    let Some(bytes) = pkg.read(&rels_name(part))? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    super::xmlx::walk(&bytes, limits, budget, |e| {
        if let super::xmlx::Ev::Start(t) = e {
            if t.name == "Relationship" {
                if let (Some(id), Some(target)) = (t.attr("Id"), t.attr("Target")) {
                    let external = t
                        .attr("TargetMode")
                        .is_some_and(|m| m.eq_ignore_ascii_case("External"));
                    out.push(Rel {
                        id: id.to_string(),
                        kind: t.attr("Type").unwrap_or("").to_string(),
                        target: resolve_target(part, target, external),
                    });
                }
            }
        }
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::testgen;

    #[test]
    fn a_normal_package_opens_and_reads_members() {
        let z = testgen::zip_of(&[("a/b.txt", b"hello"), ("c.xml", b"<x/>")]);
        let mut pkg = ZipPkg::open(&z, &Limits::default()).unwrap();
        assert!(pkg.has("a/b.txt"));
        assert_eq!(pkg.read("a/b.txt").unwrap().unwrap(), b"hello");
        assert_eq!(pkg.read("missing").unwrap(), None);
        assert_eq!(pkg.len(), 2);
    }

    #[test]
    fn a_member_over_the_cap_is_too_complex_and_never_read_whole() {
        let z = testgen::zip_bomb_member(5);
        let l = Limits {
            zip_member: 1 << 20,
            ..Limits::default()
        };
        let mut pkg = ZipPkg::open(&z, &l).unwrap();
        assert_eq!(pkg.read("bomb.bin"), Err(ExtractError::TooComplex));
    }

    #[test]
    fn the_total_cap_counts_every_member_read() {
        let z = testgen::zip_of(&[
            ("a", &vec![b'x'; 100][..]),
            ("b", &vec![b'y'; 100][..]),
            ("c", &vec![b'z'; 100][..]),
        ]);
        let l = Limits {
            zip_total: 250,
            ..Limits::default()
        };
        let mut pkg = ZipPkg::open(&z, &l).unwrap();
        assert!(pkg.read("a").is_ok());
        assert!(pkg.read("b").is_ok());
        assert_eq!(pkg.read("c"), Err(ExtractError::TooComplex));
    }

    #[test]
    fn too_many_entries_and_a_lying_directory() {
        let l = Limits::small();
        let many = testgen::zip_many_entries(51);
        assert!(matches!(
            ZipPkg::open(&many, &l),
            Err(ExtractError::TooComplex)
        ));
        assert!(ZipPkg::open(&testgen::zip_many_entries(50), &l).is_ok());
        // the end record claims 2^32 - 1 entries (zip64 marker) without a zip64 record
        let mut lie = testgen::zip_of(&[("a", b"x")]);
        let eocd = lie.len() - 22;
        lie[eocd + 10] = 0xFF;
        lie[eocd + 11] = 0xFF;
        assert!(ZipPkg::open(&lie, &Limits::default()).is_err());
        // a classic count of 60 000 entries is refused before the library allocates anything
        let mut big = testgen::zip_of(&[("a", b"x")]);
        let eocd = big.len() - 22;
        big[eocd + 10] = 0x60;
        big[eocd + 11] = 0xEA;
        assert!(matches!(
            ZipPkg::open(&big, &Limits::default()),
            Err(ExtractError::TooComplex)
        ));
    }

    #[test]
    fn a_central_directory_outside_the_input_is_corrupt() {
        let mut z = testgen::zip_of(&[("a", b"x")]);
        let eocd = z.len() - 22;
        z[eocd + 16..eocd + 20].copy_from_slice(&0x7FFF_0000u32.to_le_bytes());
        assert!(matches!(
            ZipPkg::open(&z, &Limits::default()),
            Err(ExtractError::Corrupt)
        ));
        assert!(matches!(
            ZipPkg::open(b"not a zip at all, just text", &Limits::default()),
            Err(ExtractError::Corrupt)
        ));
    }

    #[test]
    fn a_zip64_count_that_disagrees_is_corrupt() {
        let z = testgen::zip64_claiming(2, 7);
        assert!(matches!(
            ZipPkg::open(&z, &Limits::default()),
            Err(ExtractError::Corrupt)
        ));
        // a zip64 count of 2^32 entries is refused (nothing is allocated for it)
        let z = testgen::zip64_claiming(0xFFFF, 1 << 32);
        assert!(matches!(
            ZipPkg::open(&z, &Limits::default()),
            Err(ExtractError::TooComplex)
        ));
    }

    #[test]
    fn declared_sizes_are_never_trusted() {
        let z = testgen::zip_lying_size();
        match ZipPkg::open(&z, &Limits::default()) {
            Ok(mut pkg) => {
                let r = pkg.read("lie.bin");
                assert!(
                    matches!(
                        r,
                        Err(ExtractError::Corrupt) | Err(ExtractError::TooComplex)
                    ),
                    "{r:?}"
                );
            }
            Err(e) => assert!(matches!(
                e,
                ExtractError::Corrupt | ExtractError::TooComplex
            )),
        }
    }

    #[test]
    fn duplicate_names_are_corrupt() {
        let z = testgen::zip_with_duplicate_name();
        let r = ZipPkg::open(&z, &Limits::default());
        assert!(
            matches!(r, Err(ExtractError::Corrupt)),
            "{:?}",
            r.map(|p| p.len())
        );
    }

    #[test]
    fn a_part_is_read_once() {
        let z = testgen::zip_of(&[("p", b"data")]);
        let mut pkg = ZipPkg::open(&z, &Limits::default()).unwrap();
        assert_eq!(pkg.read_once("p").unwrap().unwrap(), b"data");
        assert_eq!(pkg.read_once("p").unwrap(), None);
    }

    #[test]
    fn nested_zips_are_just_bytes() {
        let inner = testgen::zip_of(&[("deep.txt", b"deep")]);
        let outer = testgen::zip_of(&[("inner.zip", &inner[..])]);
        let mut pkg = ZipPkg::open(&outer, &Limits::default()).unwrap();
        assert!(!pkg.has("deep.txt"));
        assert_eq!(pkg.read("inner.zip").unwrap().unwrap(), inner);
    }

    #[test]
    fn targets_are_resolved_inside_the_package() {
        let r = |from, t| resolve_target(from, t, false);
        assert_eq!(
            r("xl/workbook.xml", "worksheets/sheet1.xml"),
            Some("xl/worksheets/sheet1.xml".into())
        );
        assert_eq!(
            r("xl/workbook.xml", "/xl/sharedStrings.xml"),
            Some("xl/sharedStrings.xml".into())
        );
        assert_eq!(
            r("ppt/slides/slide1.xml", "../notesSlides/n1.xml"),
            Some("ppt/notesSlides/n1.xml".into())
        );
        assert_eq!(
            r("xl/workbook.xml", "worksheets/sheet%201.xml"),
            Some("xl/worksheets/sheet 1.xml".into())
        );
        // never above the root
        assert_eq!(r("xl/workbook.xml", "../../etc/passwd"), None);
        assert_eq!(r("a.xml", "../b.xml"), None);
        assert_eq!(r("xl/workbook.xml", "a/../../../x"), None);
        // links are never targets
        assert_eq!(r("xl/workbook.xml", "https://example.com/x.xml"), None);
        assert_eq!(resolve_target("xl/workbook.xml", "sheet1.xml", true), None);
        assert_eq!(r("xl/workbook.xml", ""), None);
        assert_eq!(rels_name("xl/workbook.xml"), "xl/_rels/workbook.xml.rels");
        assert_eq!(rels_name("a.xml"), "_rels/a.xml.rels");
    }
}
