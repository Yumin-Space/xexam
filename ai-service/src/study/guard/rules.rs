//! The matching rules of spec 5.8, evaluated on a normalised window.

use super::lists::*;
use super::norm::{is_cjk, is_sentence_end, norm_lower, normalize, Nc};
use crate::study::model::{QKind as Kind, StudyQuestion};

/// A lowercase pattern; Latin edges are word-bounded when `bounded`.
#[derive(Clone, Debug)]
struct Pat {
    chars: Vec<char>,
    lb: bool,
    rb: bool,
}

impl Pat {
    fn new(text: &str, bounded: bool) -> Option<Pat> {
        Pat::from_chars(norm_lower(text), bounded)
    }

    fn from_chars(chars: Vec<char>, bounded: bool) -> Option<Pat> {
        let first = *chars.first()?;
        let last = *chars.last()?;
        Some(Pat {
            lb: bounded && first.is_ascii_alphanumeric(),
            rb: bounded && last.is_ascii_alphanumeric(),
            chars,
        })
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn at(&self, cx: &Cx, i: usize) -> bool {
        let n = self.chars.len();
        if i + n > cx.s.len() {
            return false;
        }
        if self
            .chars
            .iter()
            .enumerate()
            .any(|(k, c)| cx.s[i + k].lc != *c)
        {
            return false;
        }
        (!self.lb || cx.left_free(i)) && (!self.rb || cx.right_free(i + n))
    }
}

/// A claim word of a hotspot verdict.
#[derive(Clone, Debug)]
struct Word {
    pat: Pat,
    single_cjk: bool,
}

/// The scan context: the normalised text and whether it is complete.
struct Cx<'a> {
    s: &'a [Nc],
    /// True when no more text follows (end of text counts as a boundary).
    fin: bool,
}

impl Cx<'_> {
    fn ch(&self, i: usize) -> Option<char> {
        self.s.get(i).map(|n| n.ch)
    }

    fn lc(&self, i: usize) -> Option<char> {
        self.s.get(i).map(|n| n.lc)
    }

    /// The character before `i` is not `[a-z0-9]` (or there is none).
    fn left_free(&self, i: usize) -> bool {
        i == 0 || !self.s[i - 1].lc.is_ascii_alphanumeric()
    }

    /// The character at `i` is not `[a-z0-9]`; at the end it depends on `fin`
    /// because more text might still follow.
    fn right_free(&self, i: usize) -> bool {
        match self.s.get(i) {
            Some(n) => !n.lc.is_ascii_alphanumeric(),
            None => self.fin,
        }
    }

    /// The letter at `i` is the tail of a contraction ("you'd", "he's").
    fn after_apostrophe(&self, i: usize) -> bool {
        i >= 2 && self.s[i - 1].ch == '\'' && self.s[i - 2].lc.is_ascii_alphanumeric()
    }

    /// The letter at `i` is part of a dotted abbreviation ("e.g.", "i.e.").
    fn in_abbreviation(&self, i: usize) -> bool {
        let letter = |p: usize| self.lc(p).is_some_and(|c| c.is_ascii_lowercase());
        let dot = |p: usize| self.ch(p) == Some('.');
        let first = letter(i) && dot(i + 1) && letter(i + 2) && dot(i + 3);
        let second = i >= 2 && letter(i - 2) && dot(i - 1) && letter(i) && dot(i + 1);
        first || second
    }

    /// Does `word` (lowercase) start at `i`, word-bounded on both sides?
    fn word_at(&self, i: usize, word: &str) -> Option<usize> {
        if !self.left_free(i) {
            return None;
        }
        let e = self.lit_at(i, word)?;
        self.right_free(e).then_some(e)
    }

    fn lit_at(&self, i: usize, lit: &str) -> Option<usize> {
        let n = lit.chars().count();
        if i + n > self.s.len() {
            return None;
        }
        lit.chars()
            .enumerate()
            .all(|(k, c)| self.s[i + k].lc == c)
            .then_some(i + n)
    }

    fn skip_spaces(&self, mut i: usize) -> usize {
        while self.ch(i) == Some(' ') {
            i += 1;
        }
        i
    }

    /// Is `i` the start of a line (leading bullets and spaces skipped)?
    fn line_start_at(&self, i: usize) -> bool {
        let mut j = i;
        for _ in 0..=8 {
            match self.s.get(j) {
                Some(n) if n.ls => return true,
                None => return false,
                _ => {}
            }
            if j > 0 && matches!(self.s[j - 1].ch, ' ' | '-' | '*' | '•' | '>' | '#') {
                j -= 1;
            } else {
                return false;
            }
        }
        false
    }

    /// A run of ASCII digits at `i` (not preceded by a digit): value and end.
    fn digits_at(&self, i: usize) -> Option<(usize, usize)> {
        if i > 0 && self.s[i - 1].ch.is_ascii_digit() {
            return None;
        }
        let mut j = i;
        let mut v = 0usize;
        while let Some(d) = self.ch(j).and_then(|c| c.to_digit(10)) {
            if j - i >= 3 {
                return None;
            }
            v = v * 10 + d as usize;
            j += 1;
        }
        if j == i {
            return None;
        }
        // A digit run still growing at the end of an incomplete text is open.
        if j == self.s.len() && !self.fin {
            return None;
        }
        Some((v, j))
    }
}

fn cjk_numeral(c: char) -> Option<usize> {
    "一二三四五六七八九十"
        .chars()
        .position(|x| x == c)
        .map(|p| p + 1)
}

/// Numeral one to eight: digit 1-8 or CJK 一-八.
fn small_numeral(c: char) -> Option<usize> {
    if let Some(d) = c.to_digit(10) {
        return (1..=8).contains(&d).then_some(d as usize);
    }
    cjk_numeral(c).filter(|v| *v <= 8)
}

/// Digit run or CJK numeral (one to ten) at `i`: value and end.
fn num_or_cjk_at(cx: &Cx, i: usize) -> Option<(usize, usize)> {
    if let Some(r) = cx.digits_at(i) {
        return Some(r);
    }
    cx.ch(i).and_then(cjk_numeral).map(|v| (v, i + 1))
}

/// Keeps the longest end seen.
fn longest(best: &mut Option<usize>, e: Option<usize>) {
    if let Some(e) = e {
        *best = Some(best.map_or(e, |b| b.max(e)));
    }
}

/// The compiled rules for one question.
pub struct Rules {
    kind: Kind,
    n_options: usize,
    n_items: usize,
    keys: Vec<char>,
    exempt: Vec<char>,
    leads: Vec<Pat>,
    trails: Vec<Pat>,
    x_texts: Vec<Pat>,
    quotes: Vec<Pat>,
    bare_keys: Vec<Pat>,
    words: Vec<Word>,
    loose: Vec<Pat>,
    hold: usize,
}

fn dedup_push(list: &mut Vec<Vec<char>>, v: Vec<char>) {
    if !v.is_empty() && !list.contains(&v) {
        list.push(v);
    }
}

fn contains_run(hay: &[char], needle: &[char]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

fn non_space(v: &[char]) -> usize {
    v.iter().filter(|c| !matches!(c, ' ' | '\n')).count()
}

/// Uppercase ASCII letters that stand alone in `text`.
fn standalone_letters(text: &str, out: &mut Vec<char>) {
    let n = normalize(text);
    let cx = Cx { s: &n, fin: true };
    for (i, c) in n.iter().enumerate() {
        if c.ch.is_ascii_uppercase()
            && cx.left_free(i)
            && cx.right_free(i + 1)
            && !out.contains(&c.ch)
        {
            out.push(c.ch);
        }
    }
}

impl Rules {
    /// Compiles the rules; `key_texts` are the texts of the correct option(s).
    pub fn new(q: &StudyQuestion, key_texts: &[String]) -> Rules {
        let keys: Vec<char> = q
            .options
            .iter()
            .filter_map(|o| o.key.chars().next())
            .map(|c| c.to_ascii_uppercase())
            .filter(char::is_ascii_uppercase)
            .collect();
        let mut exempt = Vec::new();
        standalone_letters(&q.stem, &mut exempt);
        for o in &q.options {
            standalone_letters(&o.text, &mut exempt);
        }
        let mut x: Vec<Vec<char>> = Vec::new();
        for o in &q.options {
            let v = norm_lower(&o.text);
            if v.len() <= X_OPTION_MAX {
                dedup_push(&mut x, v);
            }
        }
        let mut key_norm: Vec<Vec<char>> = Vec::new();
        for k in key_texts {
            dedup_push(&mut key_norm, norm_lower(k));
        }
        for k in &key_norm {
            dedup_push(&mut x, k.clone());
        }
        let quotes: Vec<Pat> = key_norm
            .iter()
            .filter(|v| {
                non_space(v) >= QUOTE_MIN_CHARS
                    || v.iter().filter(|c| is_cjk(**c)).count() >= QUOTE_MIN_CJK
            })
            .filter_map(|v| Pat::from_chars(v.clone(), false))
            .collect();

        // Rule 2b: short key texts that no other part of the question contains.
        let mut bare_keys: Vec<Pat> = Vec::new();
        if BARE_KEY_RULE && q.kind != Kind::Hotspot {
            let stem = norm_lower(&q.stem);
            let wrong: Vec<Vec<char>> = q
                .options
                .iter()
                .map(|o| norm_lower(&o.text))
                .filter(|v| !key_norm.contains(v))
                .collect();
            for v in &key_norm {
                let long_enough = non_space(v) >= BARE_KEY_MIN_CHARS
                    || v.iter().filter(|c| is_cjk(**c)).count() >= BARE_KEY_MIN_CJK;
                let yes_no = BARE_KEY_EXEMPT.contains(&v.iter().collect::<String>().as_str());
                if long_enough
                    && !yes_no
                    && !contains_run(&stem, v)
                    && !wrong.iter().any(|w| contains_run(w, v))
                {
                    bare_keys.extend(Pat::from_chars(v.clone(), true));
                }
            }
        }

        let mut words: Vec<Word> = Vec::new();
        let mut loose: Vec<Pat> = Vec::new();
        let mut choice_texts: Vec<Vec<char>> = Vec::new();
        if q.kind == Kind::Hotspot {
            for w in TRUTH_WORDS {
                words.extend(Pat::new(w, true).map(|pat| Word {
                    pat,
                    single_cjk: false,
                }));
            }
            for c in SINGLE_TRUTH {
                words.extend(Pat::from_chars(vec![*c], true).map(|pat| Word {
                    pat,
                    single_cjk: true,
                }));
            }
            for item in &q.items {
                for ch in &item.choices {
                    let v = norm_lower(ch);
                    // "db-node" is also written "db node".
                    let spaced: Vec<char> = v
                        .iter()
                        .map(|c| if matches!(c, '-' | '_') { ' ' } else { *c })
                        .collect();
                    dedup_push(&mut choice_texts, v);
                    dedup_push(&mut choice_texts, spaced);
                }
            }
            for k in &key_norm {
                dedup_push(&mut choice_texts, k.clone());
            }
            for v in &choice_texts {
                let single = v.len() == 1 && is_cjk(v[0]);
                if single && SINGLE_TRUTH.contains(&v[0]) {
                    continue; // already a word
                }
                words.extend(Pat::from_chars(v.clone(), true).map(|pat| Word {
                    pat,
                    single_cjk: single,
                }));
            }
            for v in &choice_texts {
                let s: String = v.iter().collect();
                if non_space(v) >= 3 && !TRUTH_WORDS.contains(&s.as_str()) {
                    loose.extend(Pat::from_chars(v.clone(), true));
                }
            }
        }

        let longest = x
            .iter()
            .map(Vec::len)
            .chain(choice_texts.iter().map(Vec::len))
            .chain(quotes.iter().map(Pat::len))
            .max()
            .unwrap_or(0);
        let hold = (longest + HOLD_MARGIN).clamp(HOLD_MIN, HOLD_MAX);

        Rules {
            kind: q.kind,
            n_options: q.options.len(),
            n_items: q.items.len(),
            keys,
            exempt,
            leads: LEADS.iter().filter_map(|l| Pat::new(l, true)).collect(),
            trails: TRAILS.iter().filter_map(|l| Pat::new(l, true)).collect(),
            x_texts: x
                .into_iter()
                .filter_map(|v| Pat::from_chars(v, true))
                .collect(),
            quotes,
            bare_keys,
            words,
            loose,
            hold,
        }
    }

    /// Normalised characters that are never released before they were checked.
    pub fn hold(&self) -> usize {
        self.hold
    }

    /// Checks the window `s[from..]` (earlier characters are context only).
    /// `fin` says that no more text follows.
    pub fn leaks(&self, s: &[Nc], from: usize, fin: bool) -> bool {
        let cx = Cx { s, fin };
        if self.rule2(&cx, from) {
            return true;
        }
        match self.kind {
            Kind::Single | Kind::Multi => {
                self.rule1a(&cx, from) || self.rule1b(&cx, from) || self.rule1c(&cx, from)
            }
            Kind::Hotspot => self.rule3(&cx, from),
        }
    }

    // ---- targets -------------------------------------------------------

    fn u_at(&self, cx: &Cx, i: usize) -> Option<usize> {
        let c = cx.ch(i)?;
        (self.keys.contains(&c)
            && cx.left_free(i)
            && !cx.after_apostrophe(i)
            && cx.right_free(i + 1))
        .then_some(i + 1)
    }

    fn l_at(&self, cx: &Cx, i: usize) -> Option<usize> {
        let c = cx.ch(i)?;
        if !('a'..='h').contains(&c) || !self.keys.contains(&c.to_ascii_uppercase()) {
            return None;
        }
        if !cx.left_free(i) || cx.after_apostrophe(i) || cx.in_abbreviation(i) {
            return None;
        }
        if c == 'a' {
            matches!(cx.ch(i + 1), Some(')' | '.' | ':')).then_some(i + 1)
        } else {
            cx.right_free(i + 1).then_some(i + 1)
        }
    }

    fn o_at(&self, cx: &Cx, i: usize) -> Option<usize> {
        let mut best: Option<usize> = None;
        let n = self.n_options;
        // en: <ordinal> [one filler word] option|choice|answer|one
        for (k, w) in ORDINALS_EN.iter().chain(&["last"]).enumerate() {
            let valid = *w == "last" || k < n;
            let Some(mut pos) = cx.word_at(i, w).filter(|_| valid) else {
                continue;
            };
            for _ in 0..2 {
                pos = cx.skip_spaces(pos);
                let start = pos;
                while pos - start <= 12 && cx.lc(pos).is_some_and(|c| c.is_ascii_lowercase()) {
                    pos += 1;
                }
                if pos == start {
                    break;
                }
                let word: String = (start..pos).filter_map(|p| cx.lc(p)).collect();
                if ORDINAL_NOUNS.contains(&word.as_str()) && cx.right_free(pos) {
                    longest(&mut best, Some(pos));
                    break;
                }
            }
        }
        // en: option N, choice N, #N
        for w in ["option", "choice"] {
            if let Some(p) = cx.word_at(i, w) {
                if let Some((v, e)) = cx.digits_at(cx.skip_spaces(p)) {
                    if (1..=n).contains(&v) {
                        longest(&mut best, Some(e));
                    }
                }
            }
        }
        if cx.ch(i) == Some('#') {
            if let Some((v, e)) = cx.digits_at(i + 1) {
                if (1..=n).contains(&v) {
                    longest(&mut best, Some(e));
                }
            }
        }
        // zh: 第N个?(选项|项|个|答案), 最后一个(选项)?
        if cx.ch(i) == Some('第') {
            if let Some(v) = cx.ch(i + 1).and_then(small_numeral) {
                if (1..=n).contains(&v) {
                    let e = i + 2;
                    for start in [e, e + usize::from(cx.ch(e) == Some('个'))] {
                        for alt in ["选项", "项", "个", "答案"] {
                            longest(&mut best, cx.lit_at(start, alt));
                        }
                    }
                }
            }
        }
        if let Some(e) = cx.lit_at(i, "最后一个") {
            longest(&mut best, Some(cx.lit_at(e, "选项").unwrap_or(e)));
        }
        // ja: N番目, 最初の/最後の(選択肢|もの)
        if let Some(v) = cx.ch(i).and_then(small_numeral) {
            if (1..=n).contains(&v) {
                longest(&mut best, cx.lit_at(i + 1, "番目"));
            }
        }
        for lead in ["最初の", "最後の"] {
            if let Some(e) = cx.lit_at(i, lead) {
                for alt in ["選択肢", "もの"] {
                    longest(&mut best, cx.lit_at(e, alt));
                }
            }
        }
        best
    }

    /// Ends of every target at `i` (U, O, X, and L when `with_l`).
    fn ends_at(&self, cx: &Cx, i: usize, with_l: bool) -> Vec<usize> {
        let mut ends = Vec::new();
        ends.extend(self.u_at(cx, i));
        if with_l {
            ends.extend(self.l_at(cx, i));
        }
        ends.extend(self.o_at(cx, i));
        for p in &self.x_texts {
            if p.at(cx, i) {
                ends.push(i + p.len());
            }
        }
        ends
    }

    // ---- rule 1 --------------------------------------------------------

    /// 1a: LEAD, gap(LEAD_GAP = 40), target.
    fn rule1a(&self, cx: &Cx, from: usize) -> bool {
        let len = cx.s.len();
        for i in from..len {
            for lead in &self.leads {
                if !lead.at(cx, i) {
                    continue;
                }
                let e = i + lead.len();
                for j in e..=(e + LEAD_GAP).min(len) {
                    if j > e && is_sentence_end(cx.s[j - 1].ch) {
                        break;
                    }
                    if j < len && !self.ends_at(cx, j, true).is_empty() {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// 1b: target (U, O, X), punct(6), TRAIL.
    fn rule1b(&self, cx: &Cx, from: usize) -> bool {
        for i in from..cx.s.len() {
            for t in self.ends_at(cx, i, false) {
                for k in 0..=TRAIL_PUNCT {
                    let pos = t + k;
                    if self.trails.iter().any(|tr| tr.at(cx, pos)) {
                        return true;
                    }
                    if !cx.ch(pos).is_some_and(|c| PUNCT.contains(&c)) {
                        break;
                    }
                }
            }
        }
        false
    }

    /// 1c: bare letters B..H and marked forms of any key letter.
    fn rule1c(&self, cx: &Cx, from: usize) -> bool {
        for i in from..cx.s.len() {
            let c = cx.s[i].ch;
            if !self.keys.contains(&c) {
                continue;
            }
            if c != 'A' && !self.exempt.contains(&c) && self.u_at(cx, i).is_some() {
                return true;
            }
            if self.letter_noun(cx, i, c) {
                return true;
            }
            let before = i.checked_sub(1).and_then(|p| cx.ch(p));
            let after = cx.ch(i + 1);
            let wrapped = matches!(
                (before, after),
                (Some('('), Some(')'))
                    | (Some('['), Some(']'))
                    | (Some('"'), Some('"'))
                    | (Some('“'), Some('”'))
                    | (Some('「'), Some('」'))
                    | (Some('*'), Some('*'))
            );
            // `A)` with a free left neighbour; `A）` is normalised to `A)`.
            if wrapped || (after == Some(')') && cx.left_free(i)) {
                return true;
            }
            if matches!(after, Some('.' | ':' | '、')) && cx.line_start_at(i) {
                return true;
            }
        }
        false
    }

    /// A key letter followed by an option noun ("B option", "B 选项"); checked
    /// even for a letter that stands alone in the stem.
    fn letter_noun(&self, cx: &Cx, i: usize, c: char) -> bool {
        if !cx.left_free(i) {
            return false;
        }
        let p = i + 1;
        let p = if cx.ch(p) == Some(' ') { p + 1 } else { p };
        if LETTER_NOUNS_CJK.iter().any(|n| cx.lit_at(p, n).is_some()) {
            return true;
        }
        c != 'A' && LETTER_NOUNS_EN.iter().any(|n| cx.word_at(p, n).is_some())
    }

    // ---- rule 2 --------------------------------------------------------

    /// A free-standing quote of a key text.
    fn rule2(&self, cx: &Cx, from: usize) -> bool {
        (from..cx.s.len()).any(|i| {
            self.quotes.iter().any(|p| p.at(cx, i)) || self.bare_keys.iter().any(|p| p.at(cx, i))
        })
    }

    // ---- rule 3 --------------------------------------------------------

    /// R: references to hotspot item N (1..=m); (start, end) spans.
    fn refs(&self, cx: &Cx, from: usize) -> Vec<(usize, usize)> {
        let ok = |v: usize| (1..=self.n_items).contains(&v);
        let mut out = Vec::new();
        for i in from..cx.s.len() {
            let mut best: Option<usize> = None;
            for w in ["statement", "item"] {
                if let Some(p) = cx.word_at(i, w) {
                    let q = cx.skip_spaces(p);
                    if let Some((v, e)) = cx.digits_at(q) {
                        if ok(v) {
                            longest(&mut best, Some(e));
                        }
                    }
                    for (k, nw) in NUMBER_WORDS_EN.iter().enumerate() {
                        if ok(k + 1) {
                            longest(&mut best, cx.word_at(q, nw));
                        }
                    }
                }
            }
            if cx.ch(i) == Some('#') {
                if let Some((v, e)) = cx.digits_at(i + 1) {
                    if ok(v) {
                        longest(&mut best, Some(e));
                    }
                }
            }
            if let Some(p) = cx.word_at(i, "the") {
                let p = cx.skip_spaces(p);
                for (k, w) in ORDINALS_EN_10.iter().enumerate() {
                    if let (true, Some(q)) = (ok(k + 1), cx.word_at(p, w)) {
                        let q = cx.skip_spaces(q);
                        for noun in ITEM_NOUNS {
                            longest(&mut best, cx.word_at(q, noun));
                        }
                    }
                }
            }
            // zh: 第N(项|条|句|个), 第N个(陈述|说法)
            if cx.ch(i) == Some('第') {
                if let Some((v, e)) = num_or_cjk_at(cx, i + 1) {
                    if ok(v) {
                        for alt in ["项", "条", "句", "个"] {
                            longest(&mut best, cx.lit_at(e, alt));
                        }
                        if let Some(p) = cx.lit_at(e, "个") {
                            for alt in ["陈述", "说法"] {
                                longest(&mut best, cx.lit_at(p, alt));
                            }
                        }
                    }
                }
            }
            // ja: N番目(の(文|記述|項目))?, (項目|記述)N
            if let Some((v, e)) = num_or_cjk_at(cx, i) {
                if let (true, Some(p)) = (ok(v), cx.lit_at(e, "番目")) {
                    longest(&mut best, Some(p));
                    if let Some(q) = cx.lit_at(p, "の") {
                        for alt in ["文", "記述", "項目"] {
                            longest(&mut best, cx.lit_at(q, alt));
                        }
                    }
                }
            }
            for lead in ["項目", "記述"] {
                if let Some(p) = cx.lit_at(i, lead) {
                    if let Some((v, e)) = num_or_cjk_at(cx, p) {
                        if ok(v) {
                            longest(&mut best, Some(e));
                        }
                    }
                }
            }
            // any language: (N) [N], and line-initial N. N) N、 N: N：
            if let (Some(open), Some((v, e))) = (cx.ch(i), cx.digits_at(i + 1)) {
                let close = match open {
                    '(' => Some(')'),
                    '[' => Some(']'),
                    _ => None,
                };
                if close.is_some() && cx.ch(e) == close && ok(v) {
                    longest(&mut best, Some(e + 1));
                }
            }
            if let Some((v, e)) = cx.digits_at(i) {
                if ok(v) && cx.line_start_at(i) && matches!(cx.ch(e), Some('.' | ')' | '、' | ':'))
                {
                    longest(&mut best, Some(e + 1));
                }
            }
            if let Some(e) = best {
                out.push((i, e));
            }
        }
        out
    }

    /// The context rule of a single-character CJK claim word at `i`.
    fn single_ok(&self, cx: &Cx, i: usize) -> bool {
        let preceded = i
            .checked_sub(1)
            .and_then(|p| cx.ch(p))
            .is_some_and(|c| SINGLE_PRECEDERS.contains(&c));
        if preceded {
            return true;
        }
        match cx.ch(i + 1) {
            Some(c) if CLAUSE_ENDS.contains(&c) || c == '\n' => true,
            Some(' ') => cx.s.get(i + 2).map_or(cx.fin, |n| n.ls),
            Some(_) => false,
            None => cx.fin,
        }
    }

    /// W spans (truth words, choice texts, key texts).
    fn claim_words(&self, cx: &Cx, from: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for i in from..cx.s.len() {
            for w in &self.words {
                if w.pat.at(cx, i) && (!w.single_cjk || self.single_ok(cx, i)) {
                    out.push((i, i + w.pat.len()));
                }
            }
        }
        out
    }

    /// A question about the statement, not a verdict: a question word shortly
    /// before the reference in the same clause, or `or` right after the claim
    /// word ("true or false"). An undecided end of an incomplete text counts as
    /// a question for now; the span is scanned again with more text.
    fn is_question(&self, cx: &Cx, start: usize, claim: (usize, usize)) -> bool {
        let lo = start.saturating_sub(QUESTION_LOOKBACK);
        let mut from = lo;
        for k in (lo..start).rev() {
            let c = cx.s[k].ch;
            if is_sentence_end(c) || matches!(c, ',' | ';' | ':') {
                from = k + 1;
                break;
            }
        }
        for w in QUESTION_WORDS {
            let n = w.chars().count();
            let latin = w.is_ascii();
            for k in from..start {
                if k + n > start {
                    break;
                }
                let hit = if latin {
                    cx.word_at(k, w).is_some()
                } else {
                    cx.lit_at(k, w).is_some()
                };
                if hit {
                    return true;
                }
            }
        }
        let q = cx.skip_spaces(claim.1);
        if !cx.fin && cx.s.len() - q.min(cx.s.len()) < 3 {
            return true;
        }
        QUESTION_AFTER.iter().any(|w| {
            if w.is_ascii() {
                cx.word_at(q, w).is_some()
            } else {
                cx.lit_at(q, w).is_some()
            }
        })
    }

    /// The claim word is the English `no` of "no need", "no one", ...
    fn plain_no(&self, cx: &Cx, claim: (usize, usize)) -> bool {
        let (s, e) = claim;
        if e - s != 2 || cx.lc(s) != Some('n') || cx.lc(s + 1) != Some('o') {
            return false;
        }
        let q = cx.skip_spaces(e);
        let mut p = q;
        while p - q < 12 && cx.lc(p).is_some_and(|c| c.is_ascii_lowercase()) {
            p += 1;
        }
        if p == cx.s.len() && !cx.fin {
            return true; // the next word is not complete yet
        }
        let word: String = (q..p).filter_map(|k| cx.lc(k)).collect();
        NO_NOT_A_VERDICT.contains(&word.as_str()) && cx.right_free(p)
    }

    fn rule3(&self, cx: &Cx, from: usize) -> bool {
        if self.n_items == 0 {
            return false;
        }
        let refs = self.refs(cx, from);
        if refs.is_empty() {
            return false;
        }
        // 3a: tight.
        for &(rs, re) in &refs {
            for (ws, we) in self.claim_words(cx, from) {
                let gap = if ws >= re {
                    &cx.s[re..ws]
                } else if we <= rs {
                    &cx.s[we..rs]
                } else {
                    continue;
                };
                if connectors_ok(gap)
                    && !self.is_question(cx, rs.min(ws), (ws, we))
                    && !self.plain_no(cx, (ws, we))
                {
                    return true;
                }
            }
        }
        // 3b: loose.
        for i in from..cx.s.len() {
            for p in &self.loose {
                if !p.at(cx, i) {
                    continue;
                }
                let (ps, pe) = (i, i + p.len());
                for &(rs, re) in &refs {
                    let gap = if ps >= re {
                        &cx.s[re..ps]
                    } else if pe <= rs {
                        &cx.s[pe..rs]
                    } else {
                        continue;
                    };
                    if gap.len() <= LOOSE_GAP && !gap.iter().any(|n| is_sentence_end(n.ch)) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// Is `gap` made only of connector tokens? Punctuation, spaces and symbols
/// cost one each (at most `CONNECTOR_MAX`); connector words are free but the
/// whole gap stays within `CONNECTOR_TOTAL_MAX` characters.
fn connectors_ok(gap: &[Nc]) -> bool {
    if gap.len() > CONNECTOR_TOTAL_MAX {
        return false;
    }
    let g: Vec<char> = gap.iter().map(|n| n.lc).collect();
    let words: Vec<Vec<char>> = CONNECTOR_WORDS
        .iter()
        .map(|w| w.chars().collect())
        .collect();
    let mut cost: Vec<Option<usize>> = vec![None; g.len() + 1];
    cost[0] = Some(0);
    for p in 0..g.len() {
        let Some(c) = cost[p] else { continue };
        let mut relax = |to: usize, add: usize| {
            if cost[to].is_none_or(|old| old > c + add) {
                cost[to] = Some(c + add);
            }
        };
        if PUNCT.contains(&g[p]) || CONNECTOR_CHARS.contains(&g[p]) {
            relax(p + 1, 1);
        }
        for w in &words {
            if g[p..].starts_with(w) && word_edges_ok(&g, p, w) {
                relax(p + w.len(), 0);
            }
        }
    }
    cost[g.len()].is_some_and(|c| c <= CONNECTOR_MAX)
}

/// A Latin connector word must not start or end inside a longer word; CJK
/// connectors have no word edges.
fn word_edges_ok(g: &[char], p: usize, w: &[char]) -> bool {
    let alnum = |c: char| c.is_ascii_alphanumeric();
    let left = !alnum(w[0]) || p == 0 || !alnum(g[p - 1]);
    let e = p + w.len();
    let right = !alnum(w[w.len() - 1]) || e == g.len() || !alnum(g[e]);
    left && right
}
