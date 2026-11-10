//! zip layer: `check_xlsx` and bounded member reads.
//!
//! Order and wording match the legacy `server.check_xlsx`; hardening: reject duplicate entry names, cap the entry count, always read through `take`
//! (never preallocate from the declared size), and reject any XML-like part containing DOCTYPE / ENTITY (regardless of extension case or encoding).
use std::collections::HashSet;
use std::io::{Cursor, Read};

use zip::ZipArchive;

use crate::error::ImportError;

use super::xml;

pub(super) const MAX_ENTRIES: usize = 10_000;
pub(super) const MAX_MEMBER_BYTES: u64 = 100 * 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: u64 = 300 * 1024 * 1024;
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpenError {
    NotZip,
    Duplicate,
    TooManyEntries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadError {
    Missing,
    TooBig,
    Corrupt,
}

pub(super) struct Package<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
    names: HashSet<String>,
}

impl<'a> Package<'a> {
    pub(super) fn open(data: &'a [u8]) -> Result<Self, OpenError> {
        let archive = ZipArchive::new(Cursor::new(data)).map_err(|_| OpenError::NotZip)?;
        if archive.len() > MAX_ENTRIES {
            return Err(OpenError::TooManyEntries);
        }
        if archive.has_duplicate_names() {
            return Err(OpenError::Duplicate);
        }
        let names = archive
            .file_names_raw()
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .collect();
        Ok(Self { archive, names })
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    pub(super) fn len(&self) -> usize {
        self.archive.len()
    }

    /// Name of entry `idx` (raw bytes, leniently converted to a string).
    pub(super) fn name_at(&mut self, idx: usize) -> Result<String, ReadError> {
        let f = self
            .archive
            .by_index_raw(idx)
            .map_err(|_| ReadError::Corrupt)?;
        Ok(String::from_utf8_lossy(f.name_raw()).into_owned())
    }

    /// Declared size of entry `idx`.
    pub(super) fn declared_size(&mut self, idx: usize) -> Result<u64, ReadError> {
        let f = self
            .archive
            .by_index_raw(idx)
            .map_err(|_| ReadError::Corrupt)?;
        Ok(f.size())
    }

    pub(super) fn read_index(&mut self, idx: usize) -> Result<Vec<u8>, ReadError> {
        let f = self.archive.by_index(idx).map_err(|_| ReadError::Corrupt)?;
        if f.size() > MAX_MEMBER_BYTES {
            return Err(ReadError::TooBig);
        }
        let mut buf = Vec::new();
        f.take(MAX_MEMBER_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|_| ReadError::Corrupt)?;
        if buf.len() as u64 > MAX_MEMBER_BYTES {
            return Err(ReadError::TooBig);
        }
        Ok(buf)
    }

    /// Reads a whole member by name (bounded).
    pub(super) fn read(&mut self, name: &str) -> Result<Vec<u8>, ReadError> {
        if !self.contains(name) {
            return Err(ReadError::Missing);
        }
        let idx = self
            .archive
            .index_for_name_raw(name.as_bytes())
            .ok_or(ReadError::Missing)?;
        self.read_index(idx)
    }
}

/// Upload content check (wording and order follow the legacy `server.py:check_xlsx`).
pub(super) fn check_xlsx(data: &[u8]) -> Result<(), ImportError> {
    if data.len() < 100 {
        return Err(ImportError::Empty);
    }
    if !data.starts_with(b"PK") {
        if data.starts_with(&OLE_MAGIC) {
            return Err(ImportError::LegacyXls);
        }
        return Err(ImportError::NotXlsx);
    }
    let mut pkg = Package::open(data).map_err(|e| match e {
        OpenError::TooManyEntries => ImportError::TooBigUncompressed,
        OpenError::NotZip | OpenError::Duplicate => ImportError::Corrupt,
    })?;
    if !pkg.contains("xl/workbook.xml") {
        return Err(ImportError::NoWorkbook);
    }
    let mut declared_total: u64 = 0;
    let mut actual_total: u64 = 0;
    for idx in 0..pkg.len() {
        let size = pkg.declared_size(idx).map_err(|_| ImportError::Corrupt)?;
        declared_total = declared_total.saturating_add(size);
        if size > MAX_MEMBER_BYTES || declared_total > MAX_TOTAL_BYTES {
            return Err(ImportError::TooBigUncompressed);
        }
        // The legacy version only decompressed .xml / .rels members to check them; other members (thumbnails, print settings, ...) were checked by declared size only.
        // We still read every member here to count the real bytes (the declared size is untrusted), but a read failure (e.g. a bad CRC) in a **non-XML member** does not count as a corrupt file --
        // Excel can open such files too, and the legacy version accepted them. XML members are always strict.
        let name = pkg.name_at(idx).map_err(|_| ImportError::Corrupt)?;
        let lower = name.to_ascii_lowercase();
        let is_xml_part = lower.ends_with(".xml") || lower.ends_with(".rels");
        let blob = match pkg.read_index(idx) {
            Ok(b) => b,
            Err(ReadError::TooBig) => return Err(ImportError::TooBigUncompressed),
            Err(ReadError::Corrupt) if !is_xml_part => {
                actual_total = actual_total.saturating_add(size);
                continue;
            }
            Err(ReadError::Missing | ReadError::Corrupt) => return Err(ImportError::Corrupt),
        };
        actual_total = actual_total.saturating_add(blob.len() as u64);
        if actual_total > MAX_TOTAL_BYTES {
            return Err(ImportError::TooBigUncompressed);
        }
        if xml::looks_like_xml(&blob) && xml::contains_dtd(&blob) {
            return Err(ImportError::EntityDeclaration);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_rejections_have_the_old_messages() {
        assert_eq!(check_xlsx(b"short"), Err(ImportError::Empty));
        let mut ole = OLE_MAGIC.to_vec();
        ole.resize(200, 0);
        assert_eq!(check_xlsx(&ole), Err(ImportError::LegacyXls));
        assert_eq!(check_xlsx(&[b'x'; 200]), Err(ImportError::NotXlsx));
        let mut pk = b"PK".to_vec();
        pk.resize(200, 0);
        assert_eq!(check_xlsx(&pk), Err(ImportError::Corrupt));
    }
}
