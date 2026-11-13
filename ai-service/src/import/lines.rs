//! Lines and windows of an extracted file (IA-1). Lines are numbered `L1 … Ln` (1-based, in the order of the file).
use super::{LINE_MAX_CHARS, MAX_WINDOWS, WINDOW_MAIN_CHARS, WINDOW_OVERLAP_CHARS};
use crate::extract::{Doc, FileKind};

/// The blocks of the file split at `\n`, empty lines dropped. XLSX gets a line `## <sheet name>` before each sheet's
/// rows and PPTX `## Slide <n>` before each slide (PDF page labels are not inserted: questions often cross pages). A
/// line longer than 1 000 characters is cut at the last whitespace or CJK punctuation before 1 000 (else hard).
pub fn lines(doc: &Doc) -> Vec<String> {
    let mut out = Vec::new();
    for sec in &doc.sections {
        match doc.kind {
            FileKind::Xlsx => out.push(format!("## {}", sec.label)),
            FileKind::Pptx => out.push(format!("## {}", sec.label)),
            _ => {}
        }
        for block in &sec.blocks {
            for line in block.split('\n') {
                let line = line.trim_end();
                if line.trim().is_empty() {
                    continue;
                }
                cut_line(line, &mut out);
            }
        }
    }
    out
}

fn is_break_char(c: char) -> bool {
    c.is_whitespace() || matches!(c, '。' | '！' | '？' | '；' | '，' | '、')
}

fn cut_line(line: &str, out: &mut Vec<String>) {
    let mut rest: Vec<char> = line.chars().collect();
    while rest.len() > LINE_MAX_CHARS {
        let window = &rest[..LINE_MAX_CHARS];
        let at = window
            .iter()
            .rposition(|c| is_break_char(*c))
            .map(|i| i + 1)
            .filter(|i| *i > 0)
            .unwrap_or(LINE_MAX_CHARS);
        let piece: String = rest[..at].iter().collect();
        let piece = piece.trim_end().to_string();
        if !piece.is_empty() {
            out.push(piece);
        }
        rest.drain(..at);
        while rest.first().is_some_and(|c| c.is_whitespace()) {
            rest.remove(0);
        }
    }
    let tail: String = rest.iter().collect();
    if !tail.trim().is_empty() {
        out.push(tail);
    }
}

/// A window over the lines: the question starts to report are `from ..= main_end`, `main_end + 1 ..= to` only shows
/// where the last question ends. Line numbers are 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: usize,
    pub main_end: usize,
    pub to: usize,
}

/// Total characters of the lines.
pub fn total_chars(lines: &[String]) -> usize {
    lines.iter().map(|l| l.chars().count()).sum()
}

/// The windows: consecutive lines with at most 10 000 characters in the main part (at least one line), then an overlap
/// of the following lines up to 3 000 characters; the last window has no overlap. More than 30 windows is `Err(())`
/// (the questions mode is refused: `tooMuchText`).
#[allow(clippy::result_unit_err)]
pub fn windows(lines: &[String]) -> Result<Vec<Window>, ()> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let from = i + 1;
        let mut chars = 0usize;
        let mut j = i;
        while j < lines.len() {
            let n = lines[j].chars().count();
            if j > i && chars + n > WINDOW_MAIN_CHARS {
                break;
            }
            chars += n;
            j += 1;
        }
        let main_end = j;
        let mut to = main_end;
        if main_end < lines.len() {
            let mut oc = 0usize;
            let mut k = main_end;
            while k < lines.len() {
                let n = lines[k].chars().count();
                if oc + n > WINDOW_OVERLAP_CHARS {
                    break;
                }
                oc += n;
                k += 1;
            }
            to = k;
        }
        out.push(Window { from, main_end, to });
        if out.len() > MAX_WINDOWS {
            return Err(());
        }
        i = main_end;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Section;

    fn doc(kind: FileKind, sections: Vec<(&str, Vec<&str>)>) -> Doc {
        Doc {
            kind,
            sections: sections
                .into_iter()
                .map(|(l, b)| Section {
                    label: l.to_string(),
                    blocks: b.into_iter().map(String::from).collect(),
                })
                .collect(),
            chars: 0,
            meaningful: 0,
            pages: 0,
            encoding: None,
        }
    }

    #[test]
    fn blocks_are_split_and_empty_lines_dropped() {
        let d = doc(
            FileKind::Txt,
            vec![("", vec!["one\ntwo\n\nthree", "  ", "four"])],
        );
        assert_eq!(lines(&d), vec!["one", "two", "three", "four"]);
    }

    #[test]
    fn xlsx_and_pptx_get_header_lines_pdf_does_not() {
        let x = doc(
            FileKind::Xlsx,
            vec![("Ovens", vec!["a | b"]), ("Knives", vec!["c | d"])],
        );
        assert_eq!(lines(&x), vec!["## Ovens", "a | b", "## Knives", "c | d"]);
        let p = doc(
            FileKind::Pptx,
            vec![("Slide 1", vec!["title"]), ("Slide 2", vec!["body"])],
        );
        assert_eq!(lines(&p), vec!["## Slide 1", "title", "## Slide 2", "body"]);
        let f = doc(
            FileKind::Pdf,
            vec![("p. 1", vec!["page one"]), ("p. 2", vec!["page two"])],
        );
        assert_eq!(lines(&f), vec!["page one", "page two"]);
    }

    #[test]
    fn a_long_line_is_cut_at_whitespace_before_one_thousand() {
        let words: Vec<String> = (0..300).map(|i| format!("word{i:03}")).collect();
        let long = words.join(" ");
        assert!(long.chars().count() > 1000);
        let d = doc(FileKind::Txt, vec![("", vec![long.as_str()])]);
        let l = lines(&d);
        assert!(l.len() >= 2);
        for piece in &l {
            assert!(piece.chars().count() <= 1000, "{}", piece.chars().count());
        }
        // cut on a word boundary: no piece ends or starts inside a word
        assert!(
            l.iter().all(|p| p.split(' ').all(|w| w.len() == 7)),
            "{l:?}"
        );
        assert_eq!(l.join(" "), long);
    }

    #[test]
    fn a_long_cjk_line_is_cut_after_punctuation_and_a_blob_is_hard_cut() {
        let sentence = "这是一个用来测试的句子，包含若干汉字。";
        let long = sentence.repeat(120);
        let d = doc(FileKind::Txt, vec![("", vec![long.as_str()])]);
        let l = lines(&d);
        assert!(l.len() >= 2);
        assert!(l.iter().all(|p| p.chars().count() <= 1000));
        assert!(l[0].ends_with('。') || l[0].ends_with('，'));
        let blob = "x".repeat(2500);
        let d = doc(FileKind::Txt, vec![("", vec![blob.as_str()])]);
        let l = lines(&d);
        assert_eq!(
            l.iter().map(|p| p.chars().count()).collect::<Vec<_>>(),
            vec![1000, 1000, 500]
        );
    }

    fn numbered(n: usize, each: usize) -> Vec<String> {
        (0..n)
            .map(|i| format!("{i:0width$}", width = each))
            .collect()
    }

    #[test]
    fn windows_have_a_main_part_of_ten_thousand_and_an_overlap_of_three_thousand() {
        let l = numbered(50, 1000); // 50 000 characters
        let w = windows(&l).expect("windows");
        assert_eq!(w.len(), 5);
        assert_eq!(
            w[0],
            Window {
                from: 1,
                main_end: 10,
                to: 13
            }
        );
        assert_eq!(
            w[1],
            Window {
                from: 11,
                main_end: 20,
                to: 23
            }
        );
        assert_eq!(
            w[4],
            Window {
                from: 41,
                main_end: 50,
                to: 50
            },
            "the last window has no overlap"
        );
        // every line starts in exactly one main part
        let mut next = 1;
        for win in &w {
            assert_eq!(win.from, next);
            next = win.main_end + 1;
        }
        assert_eq!(next, 51);
    }

    #[test]
    fn a_window_has_at_least_one_line_and_a_small_file_is_one_window() {
        let l = vec!["y".repeat(900); 3];
        assert_eq!(
            windows(&l).unwrap(),
            vec![Window {
                from: 1,
                main_end: 3,
                to: 3
            }]
        );
        assert_eq!(windows(&[]).unwrap(), vec![]);
        let big = vec!["z".repeat(1000); 12];
        let w = windows(&big).unwrap();
        assert_eq!(w[0].main_end, 10);
    }

    #[test]
    fn more_than_thirty_windows_are_refused() {
        let l = numbered(310, 1000); // 31 windows of 10 lines
        assert!(windows(&l).is_err());
        let l = numbered(300, 1000);
        assert_eq!(windows(&l).unwrap().len(), 30);
    }
}
