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

/// The option index of an option letter (A..F, full width too).
fn letter_index(c: char) -> Option<usize> {
    match c {
        'A'..='F' => Some(c as usize - 'A' as usize),
        'Ａ'..='Ｆ' => Some(c as usize - 'Ａ' as usize),
        _ => None,
    }
}

/// A separator inside a list of option letters at `i` ("、", ",", "/", "和", "と", " and ", " or " …): its length.
fn list_separator(v: &[char], i: usize) -> Option<usize> {
    if i < v.len()
        && matches!(
            v[i],
            '、' | ',' | '，' | '/' | '／' | '和' | '与' | '及' | 'と' | 'や' | '&' | '＆'
        )
    {
        return Some(1);
    }
    for w in ["and", "or"] {
        if let Some(len) = starts_with_ci(v, i, w) {
            let before = i == 0 || !v[i - 1].is_alphabetic();
            let after = i + len >= v.len() || !v[i + len].is_alphabetic();
            if before && after {
                return Some(len);
            }
        }
    }
    None
}

/// A bare option letter at `j`: not glued to another ASCII letter or digit on either side.
fn bare_letter(v: &[char], j: usize) -> bool {
    j < v.len()
        && is_option_letter(v[j])
        && (j == 0 || !v[j - 1].is_ascii_alphanumeric())
        && !(j + 1 < v.len() && v[j + 1].is_ascii_alphanumeric())
}

/// Positions of the letters the repair rewrites: the patterns of [`has_letter_ref`], a little stricter where a pattern
/// is also the start of an ordinary word ("A项目" is project A, "Cが正常" is "C works normally"), plus the other letters
/// of a list around a matched one ("Options A and B", "A、B项", "AとBが正しい").
fn letter_ref_positions(v: &[char]) -> Vec<usize> {
    let mut out = Vec::new();
    let letter_at = |j: usize, ascii_only: bool| {
        j < v.len()
            && is_option_letter(v[j])
            && !(j + 1 < v.len()
                && if ascii_only {
                    v[j + 1].is_ascii_alphabetic()
                } else {
                    v[j + 1].is_alphabetic()
                })
    };
    for i in 0..v.len() {
        let word_ok = i == 0 || !v[i - 1].is_alphabetic();
        if word_ok {
            for w in ["options", "option", "choice", "answer"] {
                if let Some(len) = starts_with_ci(v, i, w) {
                    let j = skip(v, i + len, |c| is_space(c) || c == ':' || c == '：');
                    if letter_at(j, false) {
                        out.push(j);
                    }
                    break;
                }
            }
        }
        if starts_with_at(v, i, "选项") {
            let j = skip(v, i + 2, is_space);
            if letter_at(j, true) {
                out.push(j);
            }
        }
        if starts_with_at(v, i, "選択肢") {
            let j = skip(v, i + 3, is_space);
            if letter_at(j, true) {
                out.push(j);
            }
        }
        if is_option_letter(v[i]) && (i == 0 || !v[i - 1].is_ascii_alphanumeric()) {
            let j = skip(v, i + 1, is_space);
            let at = |s: &str, not_next: &[char]| {
                let n = s.chars().count();
                starts_with_at(v, j, s) && !(j + n < v.len() && not_next.contains(&v[j + n]))
            };
            if at("选项", &[])
                || at("项", &['目'])
                || at("が正", &['常'])
                || at("は正", &['常'])
                || at("が誤", &[])
                || at("は誤", &[])
                || at("が不正", &[])
                || at("は不正", &[])
            {
                out.push(i);
            }
        }
    }
    // the other letters of a list around each matched one
    let mut more = Vec::new();
    for &p in &out {
        let mut k = p;
        loop {
            let a = skip(v, k + 1, is_space);
            let Some(len) = list_separator(v, a) else {
                break;
            };
            let b = skip(v, a + len, is_space);
            if !bare_letter(v, b) {
                break;
            }
            more.push(b);
            k = b;
        }
        let mut k = p;
        loop {
            // walk back over spaces, one separator, spaces, to a bare letter
            let mut a = k;
            while a > 0 && is_space(v[a - 1]) {
                a -= 1;
            }
            let sep = (1..=3).find(|&len| a >= len && list_separator(v, a - len) == Some(len));
            let Some(len) = sep else { break };
            let mut b = a - len;
            while b > 0 && is_space(v[b - 1]) {
                b -= 1;
            }
            if b == 0 || !bare_letter(v, b - 1) {
                break;
            }
            more.push(b - 1);
            k = b - 1;
        }
    }
    out.extend(more);
    out.sort_unstable();
    out.dedup();
    out
}

/// Replaces each option letter a text refers to (see [`has_letter_ref`]) by the quoted text of that option in the
/// same language (`lang`: 0 en, 1 zh, 2 ja), so the reference survives the shuffled option order. A letter without
/// such an option stays (the check still refuses it).
pub fn fix_letter_refs(s: &str, options: &[&str], lang: usize) -> String {
    let v: Vec<char> = s.chars().collect();
    let pos = letter_ref_positions(&v);
    if pos.is_empty() {
        return s.to_string();
    }
    let (open, close) = match lang {
        0 => ("\"", "\""),
        1 => ("“", "”"),
        _ => ("「", "」"),
    };
    let mut out = String::with_capacity(s.len() + 64);
    for (i, c) in v.iter().enumerate() {
        let text = pos
            .binary_search(&i)
            .ok()
            .and_then(|_| letter_index(*c))
            .and_then(|k| options.get(k))
            .map(|t| t.trim().trim_end_matches(['.', '。', '．']))
            .filter(|t| !t.is_empty());
        match text {
            Some(t) => {
                out.push_str(open);
                out.push_str(t);
                out.push_str(close);
            }
            None => out.push(*c),
        }
    }
    out
}

/// The option text without its label when the label is exactly the option's own: the letter of position `index`
/// (A, B, …; full width too) as "A. text" / "A) text" / "A、text" (punctuation then a space, or a CJK mark) or "(A) text";
/// `None` otherwise ("E. coli" at position 1, "C:/Windows", "A.I. ethics" stay as they are).
pub fn strip_own_label(s: &str, index: usize) -> Option<String> {
    let v: Vec<char> = s.trim().chars().collect();
    let own = |c: char| letter_index(c) == Some(index);
    if v.len() >= 3 && matches!(v[0], '(' | '（') && own(v[1]) && matches!(v[2], ')' | '）') {
        let rest: String = v[3..].iter().collect();
        return Some(rest.trim().to_string());
    }
    if v.len() >= 3 && own(v[0]) {
        let j = skip(&v, 1, is_space);
        if j < v.len() {
            let cjk = matches!(v[j], '．' | '、' | '：' | '）');
            let ascii = matches!(v[j], '.' | ')' | ':') && j + 1 < v.len() && is_space(v[j + 1]);
            if cjk || ascii {
                let rest: String = v[j + 1..].iter().collect();
                let rest = rest.trim().to_string();
                return (!rest.is_empty()).then_some(rest);
            }
        }
    }
    None
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
    fn letter_references_become_option_texts() {
        let en = ["Water", "Light", "Sand", "Salt"];
        let fixed = fix_letter_refs("Option B is right; option A is not.", &en, 0);
        assert_eq!(fixed, "Option \"Light\" is right; option \"Water\" is not.");
        assert!(!has_letter_ref(&fixed));
        let zh = ["水", "光", "沙", "盐"];
        let fixed = fix_letter_refs("选项B正确，C项错误。", &zh, 1);
        assert_eq!(fixed, "选项“光”正确，“沙”项错误。");
        assert!(!has_letter_ref(&fixed));
        let ja = ["水", "光", "砂", "塩"];
        let fixed = fix_letter_refs("Ｄが正しい。選択肢Aは誤り。", &ja, 2);
        assert_eq!(fixed, "「塩」が正しい。選択肢「水」は誤り。");
        assert!(!has_letter_ref(&fixed));
        // no such option: unchanged; plain words untouched
        assert_eq!(
            fix_letter_refs("Option F is odd.", &en, 0),
            "Option F is odd."
        );
        assert_eq!(
            fix_letter_refs("A plant needs light.", &en, 0),
            "A plant needs light."
        );
    }

    #[test]
    fn only_a_label_of_the_options_own_position_is_stripped() {
        assert_eq!(strip_own_label("A. Water", 0).as_deref(), Some("Water"));
        assert_eq!(strip_own_label("（C）沙子", 2).as_deref(), Some("沙子"));
        assert_eq!(strip_own_label("B) Light", 1).as_deref(), Some("Light"));
        assert_eq!(strip_own_label("Ｄ、塩", 3).as_deref(), Some("塩"));
        assert_eq!(strip_own_label("Water", 0), None);
        assert_eq!(strip_own_label("E. coli", 1), None);
        assert_eq!(strip_own_label("C:/Windows", 2), None);
        assert_eq!(strip_own_label("A.I. ethics", 0), None);
    }

    #[test]
    fn lists_of_letters_are_rewritten_whole_and_ordinary_words_stay() {
        let en = ["Water", "Light", "Sand", "Salt"];
        let zh = ["水", "光", "沙", "盐"];
        let ja = ["水", "光", "砂", "塩"];
        assert_eq!(
            fix_letter_refs("Options A and B are both correct.", &en, 0),
            "Options \"Water\" and \"Light\" are both correct."
        );
        assert_eq!(
            fix_letter_refs("选项A和B都正确。", &zh, 1),
            "选项“水”和“光”都正确。"
        );
        assert_eq!(
            fix_letter_refs("A、B项正确。", &zh, 1),
            "“水”、“光”项正确。"
        );
        assert_eq!(
            fix_letter_refs("AとBが正しい。", &ja, 2),
            "「水」と「光」が正しい。"
        );
        // project A / vitamin C are not option references
        assert_eq!(
            fix_letter_refs("A项目的净现值高于B项目。", &zh, 1),
            "A项目的净现值高于B项目。"
        );
        assert_eq!(
            fix_letter_refs("ビタミンCが正常に働く。", &ja, 2),
            "ビタミンCが正常に働く。"
        );
    }

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
