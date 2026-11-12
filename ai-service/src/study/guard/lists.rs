//! Every keyword list of the guard (spec 5.8). Each entry is exercised by a
//! positive test in `tests/lists.rs`, which iterates these constants; adding
//! an entry therefore adds a test case, and the tests also pin the lengths.

/// Words that introduce a claim about an option ("the answer is ...").
pub const LEADS: &[&str] = &[
    // en
    "answer",
    "answers",
    "correct",
    "right",
    "best",
    "choose",
    "chose",
    "select",
    "pick",
    "go with",
    "go for",
    "lean toward",
    "lean towards",
    "rule out",
    "ruled out",
    "eliminate",
    "option",
    "options",
    "choice",
    "→",
    "=>",
    "⇒",
    // zh
    "答案",
    "正确",
    "应选",
    "应该选",
    "选择",
    "选",
    "排除",
    "选项",
    "答",
    // ja
    "正解",
    "答え",
    "正しい",
    "選択肢",
    "除外",
    "選ぶ",
    "選んで",
    "選び",
];

/// Words that follow an option and pass a verdict on it.
pub const TRAILS: &[&str] = &[
    // en
    "is correct",
    "is right",
    "is the answer",
    "is the correct",
    "is the best",
    "is wrong",
    "is incorrect",
    "is not",
    "isn't",
    "is false",
    "is true",
    "is a distractor",
    "can be ruled out",
    "can be eliminated",
    "should be chosen",
    "is the right",
    "is the one",
    "is your answer",
    "is accurate",
    "are correct",
    "are right",
    "are the right",
    "are the answers",
    "are the correct",
    "are wrong",
    "are incorrect",
    // zh
    "正确",
    "是对的",
    "是错的",
    "错误",
    "不对",
    "是答案",
    "才是答案",
    "不是",
    "可以排除",
    "可排除",
    "不正确",
    "不符合",
    "是正确",
    "才是正确",
    "就是正确",
    "是正确答案",
    "就是答案",
    // ja
    "が正解",
    "は正解",
    "が正しい",
    "は正しい",
    "は誤り",
    "は間違い",
    "を選",
    "ではない",
    "は違う",
    "を除外",
    "が適切",
    "は不適切",
];

/// English ordinals for option references; `last` is the final option.
pub const ORDINALS_EN: &[&str] = &[
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth",
];

/// English ordinals for hotspot statement references (`the third statement`).
pub const ORDINALS_EN_10: &[&str] = &[
    "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth", "tenth",
];

/// Words that must follow an English ordinal within two words.
pub const ORDINAL_NOUNS: &[&str] = &[
    "option", "choice", "answer", "one", "options", "choices", "answers", "ones",
];

/// Nouns after `the <ordinal>` that make it a statement reference.
pub const ITEM_NOUNS: &[&str] = &["statement", "item", "one"];

/// Multi-character truth words (claim words, W).
pub const TRUTH_WORDS: &[&str] = &[
    // en
    "true",
    "false",
    "yes",
    "no",
    "correct",
    "incorrect",
    "right",
    "wrong",
    "accurate",
    "inaccurate",
    // zh
    "正确",
    "错误",
    "是对的",
    "是错的",
    "对的",
    "错的",
    "不对",
    "成立",
    "不成立",
    "为真",
    "为假",
    // ja
    "はい",
    "いいえ",
    "正しい",
    "誤り",
    "間違い",
    "正解",
    "不正解",
];

/// Single-character CJK truth words; they count only in a clause position.
pub const SINGLE_TRUTH: &[char] = &['是', '否', '对', '错'];

/// Characters that make a single-character CJK claim word a claim when they
/// directly precede it.
pub const SINGLE_PRECEDERS: &[char] = &['选', '答', '为', '填', ':'];

/// Characters that end a clause (a single-character claim word followed by one
/// of them, or by a line end, is a claim).
pub const CLAUSE_ENDS: &[char] = &['。', ',', '、', ';', '.', '!', '?', ')'];

/// Connector words allowed between an item reference and a claim word.
pub const CONNECTOR_WORDS: &[&str] = &[
    "is",
    "are",
    "should be",
    "the answer is",
    "choose",
    "select",
    "是",
    "应",
    "应该",
    "选",
    "答",
    "为",
    "填",
    "は",
    "が",
    "を",
    "で",
    "の答えは",
    // en: prepositions and fillers (hedges, negations) of a verdict
    "for",
    "on",
    "about",
    "re",
    "of",
    "as",
    "the",
    "not",
    "isn't",
    "aren't",
    "actually",
    "definitely",
    "also",
    "certainly",
    "really",
    "clearly",
    "answered",
    "marked",
    "holds",
    "hold",
    "so",
    "then",
    "be",
    // zh
    "说法",
    "陈述",
    "对于",
    "关于",
    // ja
    "について",
    "に関して",
    "に対して",
    "には",
    "では",
];

/// Spelled-out item numbers after `statement` / `item` (value = index + 1).
pub const NUMBER_WORDS_EN: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];

/// Words that make a sentence a question about a statement, not a verdict.
/// They count when they sit within `QUESTION_LOOKBACK` characters before the
/// reference in the same clause (no sentence end or `, ; :` in between).
pub const QUESTION_WORDS: &[&str] = &[
    "whether",
    "if",
    "or",
    "ask",
    "asks",
    "asked",
    "asking",
    "是否",
    "是不是",
    "还是",
    "如果",
    "或者",
    "询问",
    "かどうか",
    "もし",
    "または",
];

/// Words that follow `or` directly after a claim word and make it a question
/// ("true or false").
pub const QUESTION_AFTER: &[&str] = &["or", "还是", "或者", "または", "もしくは", "かどうか"];

/// Words after the claim word `no` that make it a plain negation ("no need").
pub const NO_NOT_A_VERDICT: &[&str] = &[
    "need", "one", "matter", "more", "longer", "way", "other", "rush", "worry", "problem",
];

/// Connector characters besides the punctuation set.
pub const CONNECTOR_CHARS: &[char] = &[',', '、', '→', '='];

/// Nouns after a bare key letter that make it a reference ("B option",
/// "B 选项"); the English ones are not used after `A` (an article).
pub const LETTER_NOUNS_EN: &[&str] = &["option", "choice", "answer"];
/// CJK nouns after a bare key letter (also used after `A`).
pub const LETTER_NOUNS_CJK: &[&str] = &["选项", "答案", "選択肢", "项"];

/// `punct(n)`: characters skipped between a target and a trail.
pub const PUNCT: &[char] = &[
    ' ', ':', '"', '“', '”', '\'', '‘', '’', '「', '」', '『', '』', '(', ')', '[', ']', '*', '_',
    '`', '-', '–', '—',
];

/// Longest statement of the spec: a gap contains at most this many characters
/// between a lead and its target.
pub const LEAD_GAP: usize = 40;
/// Longest run of punctuation between a target and a trail.
pub const TRAIL_PUNCT: usize = 6;
/// Most punctuation / space / symbol characters between an item reference and
/// a claim word of a tight verdict (connector words are free).
pub const CONNECTOR_MAX: usize = 8;
/// Longest whole gap of a tight verdict. The spec says "at most 8 characters",
/// which would make the 13-character connector `the answer is` unreachable.
pub const CONNECTOR_TOTAL_MAX: usize = 40;
/// Characters before a statement reference searched for a question word.
pub const QUESTION_LOOKBACK: usize = 20;
/// Longest gap of a loose statement verdict.
pub const LOOSE_GAP: usize = 20;
/// Minimum non-space characters of a free-standing quoted key text.
pub const QUOTE_MIN_CHARS: usize = 12;
/// Minimum CJK characters of a free-standing quoted key text.
pub const QUOTE_MIN_CJK: usize = 5;
/// Rule 2b (single / multi): a key text with at least this many non-space
/// characters (or CJK characters) that is in neither the stem nor a wrong option
/// text is a leak anywhere in the hint. Set `BARE_KEY_RULE` to false to turn it off.
pub const BARE_KEY_RULE: bool = true;
pub const BARE_KEY_MIN_CHARS: usize = 3;
pub const BARE_KEY_MIN_CJK: usize = 2;
/// Yes / no style key texts are left to the verdict rules (A9).
pub const BARE_KEY_EXEMPT: &[&str] = &[
    "yes",
    "no",
    "true",
    "false",
    "是",
    "否",
    "对",
    "错",
    "はい",
    "いいえ",
];
/// Option texts up to this many characters are checked as X targets.
pub const X_OPTION_MAX: usize = 120;
/// New normalised characters needed before a scan.
pub const SCAN_EVERY: usize = 32;
/// Extra window characters beyond the hold-back and the pending text.
pub const WINDOW_SLACK: usize = 48;
/// Lower / upper clamp of the hold-back and its margin over the longest text.
pub const HOLD_MIN: usize = 64;
pub const HOLD_MAX: usize = 600;
pub const HOLD_MARGIN: usize = 48;
/// Maximum characters of one hint; the run ends as truncated (one definition, in `limits`).
pub use crate::limits::HINT_MAX_CHARS;

/// Reasoning tags (lowercase; matched on the normalised text, so any letter case and full-width forms count). A leading
/// `<think>…</think>` span is removed by the OpenAI-format provider before the guard sees the text, so a tag here is
/// reasoning that reached the hint and may name the answer: the hint is withheld.
pub const THINK_TAGS: &[&str] = &["<think>", "</think>", "<thinking>", "</thinking>"];
