//! A local count of numbered questions (A-OWNER-38 item 2a): no AI call, no I/O. It lets a file with more questions than
//! the per-import limit be cut after the limit before the first recognising call (A-OWNER-47). The count is deliberately conservative: it
//! only counts numbered lines that look like a question (a question mark, a blank to fill in, or answer choices /
//! an answer line right after them), so answer keys, numbered steps and plain numbered lists do not inflate it. A
//! file that passes here is still counted exactly by the segmentation.

/// What the local scan found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocalCount {
    /// Lines that start a numbered chain of the best-fitting numbering style (`1.`, `1、`, `1)`, `(1)`, `Q1`, `第 1 题`)
    pub numbered: usize,
    /// Of those, the lines that look like a question
    pub question_like: usize,
}

/// Questions read past the limit before the text is cut, so a slightly different count of the model still reaches the limit.
pub fn cut_slack(max_questions: usize) -> usize {
    (max_questions / 10).max(2)
}

const STYLES: usize = 6;
/// Lines after a numbered line that are searched for answer choices or an answer line.
const LOOKAHEAD: usize = 8;

/// The number a line starts with in numbering style `style`, if any.
fn number_in(line: &str, style: usize) -> Option<u64> {
    let t = line.trim_start();
    let digits_at = |s: &str| -> Option<(u64, usize)> {
        let n = s.chars().take_while(|c| c.is_ascii_digit()).count();
        if n == 0 || n > 6 {
            return None;
        }
        s[..n].parse::<u64>().ok().map(|v| (v, n))
    };
    match style {
        // 1.  1．
        0 => {
            let (v, n) = digits_at(t)?;
            let mut rest = t[n..].chars();
            match rest.next()? {
                '.' | '．' if !rest.next().is_some_and(|c| c.is_ascii_digit()) => Some(v),
                _ => None,
            }
        }
        // 1、
        1 => {
            let (v, n) = digits_at(t)?;
            t[n..].starts_with('、').then_some(v)
        }
        // 1)  1）
        2 => {
            let (v, n) = digits_at(t)?;
            (t[n..].starts_with(')') || t[n..].starts_with('）')).then_some(v)
        }
        // (1)  （1）
        3 => {
            let rest = t.strip_prefix('(').or_else(|| t.strip_prefix('（'))?;
            let (v, n) = digits_at(rest)?;
            (rest[n..].starts_with(')') || rest[n..].starts_with('）')).then_some(v)
        }
        // Q1  Q.1  Q 1.  Question 1
        4 => {
            let lower: String = t.chars().take(12).collect::<String>().to_lowercase();
            let rest = if lower.starts_with("question") {
                t.get("question".len()..)?
            } else if lower.starts_with('q') {
                t.get(1..)?
            } else {
                return None;
            };
            let rest = rest.trim_start_matches(['.', ' ', '\u{3000}']);
            let (v, n) = digits_at(rest)?;
            let after = rest[n..].chars().next();
            after.is_none_or(|c| !c.is_alphanumeric()).then_some(v)
        }
        // 第1题  第 1 问
        _ => {
            let rest = t.strip_prefix('第')?.trim_start();
            let (v, n) = digits_at(rest)?;
            let tail = rest[n..].trim_start();
            ['题', '題', '问', '問']
                .iter()
                .any(|m| tail.starts_with(*m))
                .then_some(v)
        }
    }
}

/// The text after the number prefix is not needed: a line "looks like a question" by its own text or by what follows.
fn has_question_mark_or_blank(line: &str) -> bool {
    line.contains('?')
        || line.contains('？')
        || line.contains("____")
        || line.contains("（ ）")
        || line.contains("（　）")
        || line.contains("( )")
        || line.contains("（）")
        || line.contains("()")
}

fn is_choice_line(line: &str) -> bool {
    let t = line.trim_start();
    let t = t
        .strip_prefix('(')
        .or_else(|| t.strip_prefix('（'))
        .unwrap_or(t);
    let mut it = t.chars();
    let (Some(c), Some(d)) = (it.next(), it.next()) else {
        return false;
    };
    matches!(c, 'A'..='H' | 'a'..='h') && matches!(d, '.' | '、' | ')' | '）' | '．' | ':' | '：')
}

fn is_answer_line(line: &str) -> bool {
    let t = line.trim_start().to_lowercase();
    [
        "answer",
        "答案",
        "正确答案",
        "参考答案",
        "解答",
        "答え",
        "正解",
    ]
    .iter()
    .any(|k| {
        t.strip_prefix(k)
            .is_some_and(|r| r.trim_start().starts_with([':', '：']))
    })
}

/// Counts the numbered lines of the numbering style that fits best (a chain: each number is the previous one plus one,
/// or a restart at 1) and how many of them look like questions.
fn scan(lines: &[String]) -> (LocalCount, Vec<usize>) {
    let mut best = LocalCount::default();
    let mut best_like: Vec<usize> = Vec::new();
    for style in 0..STYLES {
        let mut last: Option<u64> = None;
        let mut starts: Vec<usize> = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            let Some(n) = number_in(l, style) else {
                continue;
            };
            if n == 1 || last.is_some_and(|p| n == p + 1) {
                last = Some(n);
                starts.push(i);
            }
        }
        let mut question_like = 0usize;
        let mut like_at: Vec<usize> = Vec::new();
        for (k, &i) in starts.iter().enumerate() {
            let stop = starts.get(k + 1).copied().unwrap_or(lines.len());
            let end = stop.min(i + 1 + LOOKAHEAD);
            let like = has_question_mark_or_blank(&lines[i])
                || lines[i + 1..end]
                    .iter()
                    .any(|l| is_choice_line(l) || is_answer_line(l));
            if like {
                question_like += 1;
                like_at.push(i);
            }
        }
        if question_like > best.question_like
            || (question_like == best.question_like && starts.len() > best.numbered)
        {
            best = LocalCount {
                numbered: starts.len(),
                question_like,
            };
            best_like = like_at;
        }
    }
    (best, best_like)
}

/// Counts the numbered lines of the numbering style that fits best (a chain: each number is the previous one plus one,
/// or a restart at 1) and how many of them look like questions.
pub fn count(lines: &[String]) -> LocalCount {
    scan(lines).0
}

/// Lines that open a first answer choice (`A.`, `(A)`, `a)`): about one per question that has choices, numbered or not
/// (A-OWNER-70). A second opinion on the model's count of a file whose questions carry no numbers.
pub fn first_choices(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|l| {
            let t = l.trim_start();
            let t = t
                .strip_prefix('(')
                .or_else(|| t.strip_prefix('（'))
                .unwrap_or(t);
            t.starts_with(['A', 'a']) && is_choice_line(l)
        })
        .count()
}

/// The count of the model and the local ones differ by more than this many questions: the number is shown as "about N".
pub const COUNT_TOLERANCE: usize = 3;

/// `true` when the questions found (`found`) and the local count (`max(question-like numbered lines, first choices)`)
/// disagree by more than [`COUNT_TOLERANCE`]. No local signal at all (open questions) never makes a count uncertain.
pub fn uncertain(found: usize, lines: &[String]) -> bool {
    let local = count(lines).question_like.max(first_choices(lines));
    local > 0 && found.abs_diff(local) > COUNT_TOLERANCE
}

/// How many leading lines to keep when the file has more than `max_questions` question-like lines (A-OWNER-47): the
/// text up to the question that comes `cut_slack` after the limit. `None` when the file is within the limit.
pub fn keep_lines(lines: &[String], max_questions: usize) -> Option<usize> {
    let (c, at) = scan(lines);
    let from = max_questions + cut_slack(max_questions);
    if c.question_like <= from {
        return None;
    }
    at.get(from).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// `n` made-up questions in one numbering style, each with two choices.
    fn quiz(n: usize, fmt: &dyn Fn(usize) -> String) -> Vec<String> {
        let mut v = Vec::new();
        for i in 1..=n {
            v.push(fmt(i));
            v.push("A. first".to_string());
            v.push("B. second".to_string());
        }
        v
    }

    #[test]
    fn every_numbering_style_is_counted() {
        let styles: Vec<Box<dyn Fn(usize) -> String>> = vec![
            Box::new(|i| format!("{i}. Pick one")),
            Box::new(|i| format!("{i}． Pick one")),
            Box::new(|i| format!("{i}、选一个")),
            Box::new(|i| format!("{i}) Pick one")),
            Box::new(|i| format!("({i}) Pick one")),
            Box::new(|i| format!("（{i}）选一个")),
            Box::new(|i| format!("Q{i}. Pick one")),
            Box::new(|i| format!("Question {i}: Pick one")),
            Box::new(|i| format!("第{i}题 选一个")),
            Box::new(|i| format!("第 {i} 问 选一个")),
        ];
        for (k, f) in styles.iter().enumerate() {
            let c = count(&quiz(12, f.as_ref()));
            assert_eq!((c.numbered, c.question_like), (12, 12), "style {k}");
        }
    }

    #[test]
    fn plain_lists_decimals_and_answer_keys_do_not_count_as_questions() {
        // a numbered list of facts: numbered but not question-like
        let facts: Vec<String> = (1..=30)
            .map(|i| format!("{i}. Water boils at 100 C"))
            .collect();
        let c = count(&facts);
        assert_eq!((c.numbered, c.question_like), (30, 0));
        // decimals are not numbers of a list
        assert_eq!(
            count(&l(&["3.5 kg of flour", "2.5 kg of rice"])),
            LocalCount::default()
        );
        // questions followed by a one-per-line answer key: the key does not inflate the question-like count
        let mut v = quiz(10, &|i| format!("{i}. Pick one"));
        v.extend((1..=10).map(|i| format!("{i}. B")));
        let c = count(&v);
        assert_eq!(c.question_like, 10);
        assert!(c.numbered >= 10);
    }

    #[test]
    fn question_marks_blanks_and_answer_lines_make_a_line_question_like() {
        let v = l(&[
            "1. What keeps a pot at a simmer?",
            "2. 水在标准大气压下沸点是____",
            "3. The lid keeps the heat in",
            "答案: 对",
            "4. A plain statement",
            "5. Another plain statement",
        ]);
        let c = count(&v);
        assert_eq!((c.numbered, c.question_like), (5, 3));
    }

    #[test]
    fn a_chain_needs_consecutive_numbers_or_a_restart_at_one() {
        let v = l(&["1. a?", "2. b?", "7. c?", "3. d?", "1. e?", "2. f?"]);
        assert_eq!(count(&v).numbered, 5);
    }

    #[test]
    fn keep_lines_cuts_at_the_question_after_the_limit_and_slack() {
        let v = quiz(30, &|i| format!("{i}. Pick one"));
        assert_eq!(keep_lines(&v, 100), None);
        // limit 10, slack 2: keep the lines before the 13th question (index 12, 3 lines each)
        assert_eq!(keep_lines(&v, 10), Some(36));
        assert_eq!(keep_lines(&v, 28), None);
    }

    #[test]
    fn first_choices_count_unnumbered_questions_and_uncertainty() {
        let mut v: Vec<String> = Vec::new();
        for i in 0..10 {
            v.push(format!("A stem without a number {i}"));
            for c in ["A.", "B.", "C.", "D."] {
                v.push(format!("{c} option"));
            }
        }
        assert_eq!(first_choices(&v), 10);
        assert!(!uncertain(10, &v));
        assert!(!uncertain(13, &v), "within the tolerance");
        assert!(uncertain(6, &v));
        assert!(!uncertain(6, &l(&["plain", "text"])), "no local signal");
    }
}
