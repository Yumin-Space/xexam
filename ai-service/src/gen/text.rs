//! Text helpers of the checks (GP-4): language ratios, letter references, option labels, positional options.
//! Char scanners only, no regex crate.

/// Hiragana and Katakana letters (not the middle dot U+30FB and not the long-vowel mark U+30FC).
pub fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3041..=0x3096 | 0x30A1..=0x30FA)
}

/// Han ideographs (CJK extension A, the unified block and the compatibility block).
pub fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// Length in characters (Unicode scalar values).
pub fn chars(s: &str) -> usize {
    s.chars().count()
}

struct Counts {
    letters: usize,
    ascii: usize,
    han: usize,
    kana: usize,
}

fn counts(s: &str) -> Counts {
    let mut c = Counts {
        letters: 0,
        ascii: 0,
        han: 0,
        kana: 0,
    };
    for ch in s.chars() {
        if ch.is_alphabetic() {
            c.letters += 1;
            if ch.is_ascii_alphabetic() {
                c.ascii += 1;
            }
            if is_han(ch) {
                c.han += 1;
            }
            if is_kana(ch) {
                c.kana += 1;
            }
        }
    }
    c
}

/// English: no kana, Han at most 10 % of the letters, ASCII letters at least 60 % of the letters.
pub fn looks_english(s: &str) -> bool {
    let c = counts(s);
    c.letters > 0 && c.kana == 0 && c.han * 10 <= c.letters && c.ascii * 10 >= c.letters * 6
}

/// Simplified Chinese: at least one Han character, no kana.
pub fn looks_chinese(s: &str) -> bool {
    let c = counts(s);
    c.han >= 1 && c.kana == 0
}

/// Japanese: at least one kana.
pub fn looks_japanese(s: &str) -> bool {
    counts(s).kana >= 1
}

/// An upper-case A-F or a full-width Ａ-Ｆ.
fn is_option_letter(c: char) -> bool {
    matches!(c, 'A'..='F' | 'Ａ'..='Ｆ')
}

fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\u{3000}' | '\u{00A0}' | '\t')
}

fn skip(v: &[char], mut i: usize, pred: impl Fn(char) -> bool) -> usize {
    while i < v.len() && pred(v[i]) {
        i += 1;
    }
    i
}

fn starts_with_at(v: &[char], i: usize, pat: &str) -> bool {
    let p: Vec<char> = pat.chars().collect();
    v.len() >= i + p.len() && v[i..i + p.len()] == p[..]
}

fn starts_with_ci(v: &[char], i: usize, pat: &str) -> Option<usize> {
    let p: Vec<char> = pat.chars().collect();
    if v.len() < i + p.len() {
        return None;
    }
    let ok = v[i..i + p.len()]
        .iter()
        .zip(&p)
        .all(|(a, b)| a.to_ascii_lowercase() == *b);
    ok.then_some(p.len())
}

/// A letter reference in an explanation or an option, e.g. "Option B", "answer: C", "选项 C", "C项", "Ａが正しい".
/// The letter is always upper case (a lower-case "a" is an English word).
pub fn has_letter_ref(s: &str) -> bool {
    let v: Vec<char> = s.chars().collect();
    for i in 0..v.len() {
        // English: the word, optional spaces / colons, one letter that is not followed by another letter.
        let word_ok = i == 0 || !v[i - 1].is_alphabetic();
        if word_ok {
            for w in ["options", "option", "choice", "answer"] {
                if let Some(len) = starts_with_ci(&v, i, w) {
                    let j = skip(&v, i + len, |c| is_space(c) || c == ':' || c == '：');
                    if j < v.len()
                        && is_option_letter(v[j])
                        && !(j + 1 < v.len() && v[j + 1].is_alphabetic())
                    {
                        return true;
                    }
                }
            }
        }
        // Chinese: "选项" + letter.
        if starts_with_at(&v, i, "选项") {
            let j = skip(&v, i + 2, is_space);
            if j < v.len()
                && is_option_letter(v[j])
                && !(j + 1 < v.len() && v[j + 1].is_ascii_alphabetic())
            {
                return true;
            }
        }
        // Japanese: "選択肢" + letter.
        if starts_with_at(&v, i, "選択肢") {
            let j = skip(&v, i + 3, is_space);
            if j < v.len()
                && is_option_letter(v[j])
                && !(j + 1 < v.len() && v[j + 1].is_ascii_alphabetic())
            {
                return true;
            }
        }
        // Letter first: "C项", "C 选项", "Ａが正", ...
        if is_option_letter(v[i]) && (i == 0 || !v[i - 1].is_ascii_alphanumeric()) {
            let j = skip(&v, i + 1, is_space);
            for suffix in [
                "选项",
                "项",
                "が正",
                "は正",
                "が誤",
                "は誤",
                "が不正",
                "は不正",
            ] {
                if starts_with_at(&v, j, suffix) {
                    return true;
                }
            }
        }
    }
    false
}

/// An option text that starts with its own label ("A. ...", "B) ...", "（C）...").
pub fn has_option_label(s: &str) -> bool {
    let v: Vec<char> = s.trim().chars().collect();
    if v.is_empty() {
        return false;
    }
    if is_option_letter(v[0]) {
        let j = skip(&v, 1, is_space);
        return j < v.len() && matches!(v[j], '.' | ')' | '．' | '、' | ':' | '：' | '）');
    }
    if matches!(v[0], '(' | '（')
        && v.len() >= 3
        && is_option_letter(v[1])
        && matches!(v[2], ')' | '）')
    {
        return true;
    }
    false
}

/// Options like "all of the above" / "以上都对" (C7).
pub fn has_positional(s: &str) -> bool {
    const PATTERNS: [&str; 12] = [
        "all of the above",
        "none of the above",
        "both of the above",
        "以上都",
        "以上皆",
        "以上均",
        "以上全",
        "都不对",
        "上記のすべて",
        "上記のいずれ",
        "上記の全て",
        "いずれも",
    ];
    let low = s.to_lowercase();
    PATTERNS.iter().any(|p| low.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kana_and_han_ranges() {
        assert!(is_kana('あ') && is_kana('ア') && is_kana('ヺ'));
        assert!(!is_kana('・') && !is_kana('ー') && !is_kana('中'));
        assert!(is_han('中') && is_han('㐀') && is_han('豈'));
        assert!(!is_han('あ') && !is_han('a'));
    }

    #[test]
    fn english_ratios_with_technical_terms() {
        assert!(looks_english(
            "Which setting keeps the steam oven from drying the bread?"
        ));
        // A few Han characters (a product name) stay within 10 % of the letters.
        assert!(looks_english(
            "Why does the 蒸气 steam oven door lock after the pre-heat cycle ends early today?"
        ));
        assert!(!looks_english("蒸气烤箱的门为什么会锁住"));
        assert!(!looks_english("オーブンのドアが開かない"));
        assert!(!looks_english("12345 !!!"));
        assert!(!looks_english("Ovenのdoorがopenしない why"));
    }

    #[test]
    fn chinese_and_japanese_detection() {
        assert!(looks_chinese("蒸气烤箱的门为什么会锁住 (SKU-12)"));
        assert!(!looks_chinese("Steam oven"));
        assert!(!looks_chinese("蒸気オーブンのドア"));
        assert!(looks_japanese("蒸気オーブンのドアが開かない"));
        assert!(!looks_japanese("蒸气烤箱的门"));
        assert!(!looks_japanese("ー・"));
    }

    #[test]
    fn letter_reference_positives() {
        for s in [
            "Option B is wrong because the oven stays cold.",
            "The answer: C fits the case.",
            "Choice D is too slow",
            "options E and F are unrelated",
            "选项 C 不对",
            "选项C不对",
            "C项错误",
            "C 选项错误",
            "選択肢 B が誤り",
            "Ａが正しい",
            "Bは誤りです",
            "Dが不正解です",
            "OPTION A",
        ] {
            assert!(has_letter_ref(s), "{s}");
        }
    }

    #[test]
    fn letter_reference_negatives() {
        for s in [
            "Plan A is cheaper",
            "The IPv6 address is fixed",
            "the answer a user gives is stored",
            "option a is not a reference",
            "Optional extras cost more",
            "The suboption B lists nothing",
            "Use model B2 for steam",
            "选择蒸箱，不选烤箱",
            "项目A很便宜",
            "Answers vary by region",
        ] {
            assert!(!has_letter_ref(s), "{s}");
        }
    }

    #[test]
    fn option_label_positives_and_negatives() {
        for s in [
            "A. Steam oven",
            "B) Induction hob",
            "（C）蒸气",
            "(D) 火",
            "Ｅ．冷蔵庫",
            "F: Freezer",
            "A、オーブン",
        ] {
            assert!(has_option_label(s), "{s}");
        }
        for s in [
            "A/B testing",
            "B2B order",
            "Apple press",
            "A",
            "Steam oven",
            "(G) x",
            "",
        ] {
            assert!(!has_option_label(s), "{s}");
        }
    }

    #[test]
    fn positional_options() {
        for s in [
            "All of the above",
            "NONE OF THE ABOVE",
            "Both of the above",
            "以上都对",
            "以上皆非",
            "以上均不正确",
            "以上全部",
            "都不对",
            "上記のすべて",
            "上記のいずれでもない",
            "上記の全て",
            "いずれも誤り",
        ] {
            assert!(has_positional(s), "{s}");
        }
        assert!(!has_positional("A steam oven"));
        assert!(!has_positional("蒸气烤箱"));
    }
}
