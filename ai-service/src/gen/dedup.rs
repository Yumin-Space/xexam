//! De-duplication (GP-5): no copy of a source stem, no repeat inside the job, no repeat of an earlier AI bank.

use std::collections::HashSet;

use crate::gen::check::Reject;
use crate::gen::{Tri, SOURCE_STEM_CHARS};

pub const COPY_SIM: f64 = 0.6;
pub const DUP_SIM: f64 = 0.5;
/// The small-source rule's threshold: the n-gram sets are the same (the same stem up to case and punctuation).
pub const SAME_SIM: f64 = 0.95;

/// Lower-case and keep only alphanumeric characters (so punctuation, spaces and case never hide a copy).
pub fn norm(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Character n-grams of `norm(s)`; shorter than `n` gives `{norm(s)}` (nothing when that is empty).
pub fn grams(s: &str, n: usize) -> HashSet<String> {
    let chars: Vec<char> = norm(s).chars().collect();
    let mut out = HashSet::new();
    if chars.is_empty() || n == 0 {
        return out;
    }
    if chars.len() < n {
        out.insert(chars.iter().collect());
        return out;
    }
    for w in chars.windows(n) {
        out.insert(w.iter().collect());
    }
    out
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = (a.len() + b.len()) as f64 - inter;
    inter / union
}

/// Similarity of two stems: the larger of the Jaccard index of the English 4-grams and of the Chinese 2-grams.
pub fn similarity(a: &Tri, b: &Tri) -> f64 {
    let en = jaccard(&grams(&a.en, 4), &grams(&b.en, 4));
    let zh = jaccard(&grams(&a.zh, 2), &grams(&b.zh, 2));
    en.max(zh)
}

/// The n-gram sets of a stem, computed once.
struct Stem {
    en: HashSet<String>,
    zh: HashSet<String>,
}

impl Stem {
    fn of(t: &Tri) -> Stem {
        Stem {
            en: grams(&t.en, 4),
            zh: grams(&t.zh, 2),
        }
    }

    fn sim(&self, o: &Stem) -> f64 {
        jaccard(&self.en, &o.en).max(jaccard(&self.zh, &o.zh))
    }
}

fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn cut_tri(t: &Tri, n: usize) -> Tri {
    Tri {
        en: cut(&t.en, n),
        zh: cut(&t.zh, n),
        ja: cut(&t.ja, n),
    }
}

fn option_key(options: &[Tri]) -> Vec<String> {
    let mut v: Vec<String> = options.iter().map(|o| norm(&o.en)).collect();
    v.sort();
    v
}

/// Holds the source stems, the stems of earlier AI banks and everything accepted so far in the job.
pub struct Deduper {
    source: Vec<Stem>,
    previous: Vec<Stem>,
    accepted: Vec<Stem>,
    option_sets: Vec<Vec<String>>,
    /// Small-source rule (A-OWNER-31): fewer sources than the goal, so "too close" means the same stem (or the same
    /// option set) only, never a near match
    relaxed: bool,
}

impl Deduper {
    /// `source`: every source stem of API-I1 and every seed stem (cut to `SOURCE_STEM_CHARS` here, so a long seed
    /// stem compares like a listed one); `previous`: API-I1 `previous` (empty until built).
    pub fn new(source: Vec<Tri>, previous: Vec<Tri>) -> Deduper {
        Deduper {
            source: source
                .iter()
                .map(|t| Stem::of(&cut_tri(t, SOURCE_STEM_CHARS)))
                .collect(),
            previous: previous.iter().map(Stem::of).collect(),
            accepted: Vec::new(),
            option_sets: Vec::new(),
            relaxed: false,
        }
    }

    /// Switches to the small-source rule: only an identical stem counts as a copy or a duplicate.
    pub fn relaxed(mut self, on: bool) -> Deduper {
        self.relaxed = on;
        self
    }

    /// `Copy` when the stem (cut to 160 characters) is too close to a source stem; `Duplicate` when it is too close to
    /// an accepted or earlier-bank stem, or its English option set equals an accepted question's.
    pub fn check(&self, stem: &Tri, options: &[Tri]) -> Result<(), Reject> {
        let cut_stem = Stem::of(&cut_tri(stem, SOURCE_STEM_CHARS));
        let (copy_at, dup_at) = if self.relaxed {
            (SAME_SIM, SAME_SIM)
        } else {
            (COPY_SIM, DUP_SIM)
        };
        if self.source.iter().any(|s| cut_stem.sim(s) >= copy_at) {
            return Err(Reject::Copy);
        }
        let full = Stem::of(stem);
        if self
            .accepted
            .iter()
            .chain(self.previous.iter())
            .any(|s| full.sim(s) >= dup_at)
        {
            return Err(Reject::Duplicate);
        }
        if self.option_sets.contains(&option_key(options)) {
            return Err(Reject::Duplicate);
        }
        Ok(())
    }

    pub fn accept(&mut self, stem: &Tri, options: &[Tri]) {
        self.accepted.push(Stem::of(stem));
        self.option_sets.push(option_key(options));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::testkit::*;

    fn stem(en: &str, zh: &str) -> Tri {
        Tri::new(en, zh, "x")
    }

    fn opts(words: &[&str]) -> Vec<Tri> {
        words.iter().map(|w| Tri::new(w, w, w)).collect()
    }

    #[test]
    fn norm_and_grams() {
        assert_eq!(norm("Hello, World 42!"), "helloworld42");
        assert_eq!(norm("蒸气 烤箱！"), "蒸气烤箱");
        assert_eq!(grams("abcdef", 4).len(), 3);
        assert_eq!(grams("ab", 4), std::iter::once("ab".to_string()).collect());
        assert!(grams("!!", 4).is_empty());
    }

    #[test]
    fn similarity_bounds() {
        let a = stem(
            "Which steam oven setting keeps bread moist during a long bake?",
            "哪种蒸烤设置能让面包在长时间烘烤时保持湿润",
        );
        assert!((similarity(&a, &a) - 1.0).abs() < 1e-9);
        let b = stem("", "");
        assert_eq!(similarity(&a, &b), 0.0);
    }

    #[test]
    fn a_reworded_copy_is_caught() {
        let source = stem(
            "Which steam oven setting keeps bread moist during a long bake?",
            "哪种蒸烤设置能让面包在长时间烘烤时保持湿润？",
        );
        let d = Deduper::new(vec![source], vec![]);
        let reworded = stem(
            "Which steam oven setting keeps the bread moist during a long baking?",
            "哪种蒸烤设置能让面包在较长时间烘烤时保持湿润？",
        );
        assert_eq!(d.check(&reworded, &opts(&["a1", "b2"])), Err(Reject::Copy));
    }

    #[test]
    fn a_new_scenario_on_the_same_topic_passes() {
        let source = stem(
            "Which steam oven setting keeps bread moist during a long bake?",
            "哪种蒸烤设置能让面包在长时间烘烤时保持湿润？",
        );
        let d = Deduper::new(vec![source], vec![]);
        let fresh = stem(
            "A bakery notices dry crusts after switching to a smaller combi oven; what should the baker adjust first?",
            "一家面包店换了较小的万能蒸烤箱后发现面包皮发干，烘焙师应该先调整什么？",
        );
        assert_eq!(d.check(&fresh, &opts(&["a1", "b2"])), Ok(()));
    }

    #[test]
    fn source_stems_are_compared_cut_to_160_characters() {
        let long_tail = "x".repeat(300);
        let src = stem(
            &format!("Which steam oven setting keeps bread moist during a long bake {long_tail}"),
            "z",
        );
        let d = Deduper::new(vec![src], vec![]);
        let cand = stem(
            "Which steam oven setting keeps bread moist during a long bake",
            "y",
        );
        assert_eq!(d.check(&cand, &opts(&["a1"])), Err(Reject::Copy));
    }

    #[test]
    fn accepted_stems_block_near_repeats() {
        let mut d = Deduper::new(vec![], vec![]);
        let first = tri(1, 10);
        assert_eq!(d.check(&first, &opts(&["p1", "q2"])), Ok(()));
        d.accept(&first, &opts(&["p1", "q2"]));
        assert_eq!(
            d.check(&first, &opts(&["r3", "s4"])),
            Err(Reject::Duplicate)
        );
        let other = tri(2, 10);
        assert_eq!(d.check(&other, &opts(&["r3", "s4"])), Ok(()));
    }

    #[test]
    fn identical_option_sets_are_duplicates() {
        let mut d = Deduper::new(vec![], vec![]);
        d.accept(&tri(1, 10), &opts(&["Gas", "Steam", "Induction"]));
        // A different stem, the same options in another order and case.
        assert_eq!(
            d.check(&tri(2, 10), &opts(&["induction", "GAS", "steam"])),
            Err(Reject::Duplicate)
        );
        assert_eq!(
            d.check(&tri(2, 10), &opts(&["induction", "gas", "wood"])),
            Ok(())
        );
    }

    #[test]
    fn a_near_repeat_of_a_previous_stem_is_a_duplicate() {
        let prev = stem(
            "Why does a convection oven cook a tray of rolls faster than a static oven?",
            "为什么对流烤箱烤一盘小面包比普通烤箱更快？",
        );
        let d = Deduper::new(vec![], vec![prev]);
        let again = stem(
            "Why does a convection oven cook a tray of rolls faster than the static oven?",
            "为什么对流烤箱烤一盘小面包比静态烤箱更快？",
        );
        assert_eq!(d.check(&again, &opts(&["a1"])), Err(Reject::Duplicate));
    }

    #[test]
    fn thresholds_are_pinned() {
        assert_eq!(COPY_SIM, 0.6);
        assert_eq!(DUP_SIM, 0.5);
    }

    /// A-OWNER-31: with fewer sources than the goal only the same stem is "too close"
    #[test]
    fn a_small_source_relaxes_near_matches_but_not_the_same_stem() {
        let source = stem(
            "Which steam oven setting keeps bread moist during a long bake?",
            "哪种蒸烤设置能让面包在长时间烘烤时保持湿润？",
        );
        let reworded = stem(
            "Which steam oven setting keeps the bread moist during a long baking?",
            "哪种蒸烤设置能让面包在较长时间烘烤时保持湿润？",
        );
        let o = opts(&["a", "b", "c", "d"]);
        let strict = Deduper::new(vec![source.clone()], vec![]);
        assert_eq!(strict.check(&reworded, &o), Err(Reject::Copy));
        let mut relaxed = Deduper::new(vec![source.clone()], vec![]).relaxed(true);
        assert_eq!(relaxed.check(&reworded, &o), Ok(()));
        assert_eq!(relaxed.check(&source, &o), Err(Reject::Copy));
        relaxed.accept(&reworded, &o);
        assert_eq!(
            relaxed.check(&reworded, &opts(&["e", "f", "g", "h"])),
            Err(Reject::Duplicate)
        );
        assert_eq!(
            relaxed.check(
                &stem(
                    "Other topic entirely about knives and sharpening",
                    "关于刀具和磨刀的另一个话题"
                ),
                &o
            ),
            Err(Reject::Duplicate),
            "the same option set stays a duplicate"
        );
    }
}
