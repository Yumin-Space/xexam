//! zip 层：`check_xlsx` 与带上限的成员读取。
//!
//! 与旧版 `server.check_xlsx` 的顺序和文案一致；加固：拒绝重复条目名、限制条目数、读取一律 `take`
//! （绝不按声明大小预分配）、任何像 XML 的部件含 DOCTYPE / ENTITY 都拒绝（不看扩展名大小写，也不看编码）。
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

    /// 第 `idx` 个条目的声明大小。
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

    /// 按名字读整个成员（有界）。
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

/// 上传内容检查（文案与顺序见旧版 `server.py:check_xlsx`）。
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
        let blob = pkg.read_index(idx).map_err(|e| match e {
            ReadError::TooBig => ImportError::TooBigUncompressed,
            ReadError::Missing | ReadError::Corrupt => ImportError::Corrupt,
        })?;
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
