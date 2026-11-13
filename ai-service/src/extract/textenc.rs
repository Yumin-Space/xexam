//! Decoding of text files (IX-4). A byte order mark decides; else valid UTF-8; else the guesses in the order of the UI
//! language hint; else windows-1252 when no byte it leaves undefined is present; else `BadEncoding`.
use super::{ExtractError, LangHint};

/// The decoded text and the name of the encoding that read it.
pub fn decode(bytes: &[u8], hint: LangHint) -> Result<(String, &'static str), ExtractError> {
    let (text, name) = decode_inner(bytes, hint)?;
    if text.contains('\0') {
        return Err(ExtractError::Unsupported);
    }
    Ok((text, name))
}

fn decode_inner(bytes: &[u8], hint: LangHint) -> Result<(String, &'static str), ExtractError> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return match std::str::from_utf8(rest) {
            Ok(s) => Ok((s.to_string(), "utf-8")),
            Err(_) => Err(ExtractError::BadEncoding),
        };
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, false);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, true);
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok((s.to_string(), "utf-8"));
    }
    let order: [(&'static encoding_rs::Encoding, &'static str); 2] = match hint {
        LangHint::Ja => [
            (encoding_rs::SHIFT_JIS, "shift_jis"),
            (encoding_rs::GB18030, "gb18030"),
        ],
        _ => [
            (encoding_rs::GB18030, "gb18030"),
            (encoding_rs::SHIFT_JIS, "shift_jis"),
        ],
    };
    for (enc, name) in order {
        if let Some(s) = enc.decode_without_bom_handling_and_without_replacement(bytes) {
            return Ok((s.into_owned(), name));
        }
    }
    // windows-1252 leaves 0x81, 0x8D, 0x8F, 0x90 and 0x9D undefined
    if !bytes
        .iter()
        .any(|b| matches!(b, 0x81 | 0x8D | 0x8F | 0x90 | 0x9D))
    {
        let (s, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
        return Ok((s.into_owned(), "windows-1252"));
    }
    Err(ExtractError::BadEncoding)
}

fn utf16(rest: &[u8], big: bool) -> Result<(String, &'static str), ExtractError> {
    let (pairs, tail) = rest.as_chunks::<2>();
    if !tail.is_empty() {
        return Err(ExtractError::BadEncoding);
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
    match String::from_utf16(&units) {
        Ok(s) => Ok((s, if big { "utf-16be" } else { "utf-16le" })),
        Err(_) => Err(ExtractError::BadEncoding),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::testgen;

    const SAMPLE_ZH: &str = "请问哪种工具可以让汤锅保持稳定的小火炖煮？";
    const SAMPLE_JA: &str = "ストックポットを一定の弱火で保つ道具はどれですか。";

    #[test]
    fn utf8_with_and_without_a_bom() {
        assert_eq!(
            decode("plain é".as_bytes(), LangHint::En).unwrap().1,
            "utf-8"
        );
        let (t, n) = decode(&testgen::utf8_bom("bom text"), LangHint::En).unwrap();
        assert_eq!((t.as_str(), n), ("bom text", "utf-8"));
    }

    #[test]
    fn utf16_with_a_bom() {
        let (t, n) = decode(&testgen::utf16le_bom("hello 世界"), LangHint::En).unwrap();
        assert_eq!((t.as_str(), n), ("hello 世界", "utf-16le"));
        let (t, n) = decode(&testgen::utf16be_bom("hello"), LangHint::En).unwrap();
        assert_eq!((t.as_str(), n), ("hello", "utf-16be"));
        assert_eq!(
            decode(&[0xFF, 0xFE, 0x41], LangHint::En),
            Err(ExtractError::BadEncoding)
        );
    }

    #[test]
    fn legacy_chinese_and_japanese_follow_the_hint() {
        let gb = testgen::gb18030(SAMPLE_ZH);
        let (t, n) = decode(&gb, LangHint::Zh).unwrap();
        assert_eq!((t.as_str(), n), (SAMPLE_ZH, "gb18030"));
        let sj = testgen::shift_jis(SAMPLE_JA);
        let (t, n) = decode(&sj, LangHint::Ja).unwrap();
        assert_eq!((t.as_str(), n), (SAMPLE_JA, "shift_jis"));
    }

    #[test]
    fn the_hint_orders_the_guesses() {
        // bytes valid in both: the hint decides which one reads them
        let bytes = [0xB0, 0xA1, 0xB0, 0xA2];
        assert_eq!(decode(&bytes, LangHint::Zh).unwrap().1, "gb18030");
        assert_eq!(decode(&bytes, LangHint::En).unwrap().1, "gb18030");
        assert_eq!(decode(&bytes, LangHint::Ja).unwrap().1, "shift_jis");
    }

    #[test]
    fn windows_1252_and_bad_encodings() {
        let (t, n) = decode(&[b'c', b'a', b'f', 0xE9], LangHint::En).unwrap();
        assert_eq!((t.as_str(), n), ("café", "windows-1252"));
        // 0x81 is undefined in windows-1252; a lone one is not valid in the other two either
        assert_eq!(
            decode(&[b'a', 0x81], LangHint::En),
            Err(ExtractError::BadEncoding)
        );
    }

    #[test]
    fn a_nul_byte_is_unsupported() {
        assert_eq!(
            decode(b"ab\0cd", LangHint::En),
            Err(ExtractError::Unsupported)
        );
    }
}
