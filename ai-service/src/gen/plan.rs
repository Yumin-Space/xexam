//! The plan of a generation (GP-1): how many questions per sheet, which types, how many options, which answer
//! positions, which difficulty and which source question each slot is modelled on. Pure and deterministic: a seed
//! gives exactly one plan.
//!
//! Random draws are made in the step order P2 (only when there are more sheets than questions), P5, P6, P7, P8, P9;
//! every shuffle starts from the slots' creation order (sheet order, singles before multis inside a sheet).

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::gen::rng::Rng;
use crate::gen::{
    Composition, Diff, Kind, SourceItem, Stratum, EST_CHARS_PER_QUESTION, MAX_GOAL, MAX_PER_CALL,
};

/// Everything the planner needs.
pub struct PlanInput<'a> {
    pub items: &'a [SourceItem],
    pub strata: &'a [Stratum],
    pub comp: Composition,
    pub per_call: usize,
    pub seed: u64,
}

/// One question to write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// 1-based final position.
    pub no: usize,
    pub stratum: usize,
    pub kind: Kind,
    pub opts: u8,
    /// Sorted 0-based positions of the correct options.
    pub correct: Vec<u8>,
    /// Id of the source question the slot is modelled on.
    pub seed: String,
    pub diff: Option<Diff>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// Ordered by `no`.
    pub slots: Vec<Slot>,
    /// Indexes into `slots`, consecutive chunks of `per_call`.
    pub batches: Vec<Vec<usize>>,
    /// The `per_call` the batches were cut with (at least 1).
    pub per_call: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanError {
    NoSource,
    BadComposition,
}

/// Largest-remainder apportionment of `total` over `weights`; remainder ties are broken by `tie` (Less = first).
fn hamilton(total: usize, weights: &[u64], tie: &dyn Fn(usize, usize) -> Ordering) -> Vec<usize> {
    let sum: u64 = weights.iter().sum();
    let mut out = vec![0usize; weights.len()];
    if sum == 0 || total == 0 {
        return out;
    }
    let mut rems: Vec<(usize, u64)> = Vec::new();
    let mut given = 0usize;
    for (i, &w) in weights.iter().enumerate() {
        let prod = total as u64 * w;
        out[i] = (prod / sum) as usize;
        given += out[i];
        rems.push((i, prod % sum));
    }
    rems.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| tie(a.0, b.0)));
    for &(i, _) in rems.iter().take(total - given) {
        out[i] += 1;
    }
    out
}

/// The most frequent value; ties: 4 when it is among them, else the smaller.
fn mode(values: impl Iterator<Item = u8>) -> Option<u8> {
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    for v in values {
        *counts.entry(v).or_insert(0) += 1;
    }
    let best = counts.values().copied().max()?;
    let tied: Vec<u8> = counts
        .iter()
        .filter(|(_, &c)| c == best)
        .map(|(&v, _)| v)
        .collect();
    if tied.contains(&4) {
        Some(4)
    } else {
        tied.first().copied()
    }
}

struct Draft {
    stratum: usize,
    kind: Kind,
    opts: u8,
    correct: Vec<u8>,
    diff: Option<Diff>,
    seed: String,
}

/// P8: the least-used candidate of the first non-empty tier (ties: seeded random); `excluded` ids are never chosen.
fn pick_seed(
    stratum: usize,
    kind: Kind,
    diff: Option<Diff>,
    items: &[SourceItem],
    used: &HashMap<String, usize>,
    excluded: &HashSet<String>,
    rng: &mut Rng,
) -> Option<String> {
    let present = |it: &&SourceItem| !excluded.contains(&it.id);
    let in_stratum: Vec<&SourceItem> = items
        .iter()
        .filter(present)
        .filter(|it| it.stratum == stratum)
        .collect();
    let tiers: Vec<Vec<&SourceItem>> = vec![
        in_stratum
            .iter()
            .copied()
            .filter(|it| it.kind == kind && (diff.is_none() || it.diff == diff))
            .collect(),
        in_stratum
            .iter()
            .copied()
            .filter(|it| it.kind == kind)
            .collect(),
        in_stratum
            .iter()
            .copied()
            .filter(|it| diff.is_some() && it.diff == diff)
            .collect(),
        in_stratum.clone(),
        // Fallback 4: the stratum has no present item at all.
        items.iter().filter(present).collect(),
    ];
    let tier = tiers.into_iter().find(|t| !t.is_empty())?;
    let least = tier
        .iter()
        .map(|it| used.get(&it.id).copied().unwrap_or(0))
        .min()?;
    let tied: Vec<&&SourceItem> = tier
        .iter()
        .filter(|it| used.get(&it.id).copied().unwrap_or(0) == least)
        .collect();
    let at = if tied.len() > 1 {
        rng.below(tied.len())
    } else {
        0
    };
    Some(tied[at].id.clone())
}

/// Builds the plan (P1 .. P10).
pub fn plan(i: &PlanInput) -> Result<Plan, PlanError> {
    let goal = i.comp.single + i.comp.multi;
    if !(1..=MAX_GOAL).contains(&goal) {
        return Err(PlanError::BadComposition);
    }
    let sn = i.strata.len();
    let items: Vec<SourceItem> = i
        .items
        .iter()
        .filter(|it| it.stratum < sn)
        .cloned()
        .collect();
    if items.is_empty() {
        return Err(PlanError::NoSource);
    }
    // Sheets without questions take no part (they cannot supply a seed).
    let active: Vec<usize> = (0..sn).filter(|&s| i.strata[s].n > 0).collect();
    if active.is_empty() {
        return Err(PlanError::NoSource);
    }
    let mut rng = Rng::new(i.seed);

    // P2: quotas per sheet.
    let mut q = vec![0usize; sn];
    if active.len() <= goal {
        let weights: Vec<u64> = (0..sn)
            .map(|s| {
                if i.strata[s].n > 0 {
                    i.strata[s].n as u64
                } else {
                    0
                }
            })
            .collect();
        let tie = |a: usize, b: usize| i.strata[b].n.cmp(&i.strata[a].n).then(a.cmp(&b));
        q = hamilton(goal, &weights, &tie);
        while let Some(zero) = active
            .iter()
            .copied()
            .filter(|&s| q[s] == 0)
            .min_by(|&a, &b| i.strata[b].n.cmp(&i.strata[a].n).then(a.cmp(&b)))
        {
            let donor = active
                .iter()
                .copied()
                .filter(|&s| q[s] >= 2)
                .min_by(|&a, &b| {
                    q[b].cmp(&q[a])
                        .then(i.strata[b].n.cmp(&i.strata[a].n))
                        .then(b.cmp(&a))
                });
            let Some(donor) = donor else { break };
            q[donor] -= 1;
            q[zero] += 1;
        }
    } else {
        let mut order = active.clone();
        rng.shuffle(&mut order);
        order.sort_by(|&a, &b| i.strata[b].n.cmp(&i.strata[a].n));
        for &s in order.iter().take(goal) {
            q[s] = 1;
        }
    }

    // P3: types per sheet (A0): sheets with multiple-choice sources are preferred.
    let w: Vec<u64> = (0..sn)
        .map(|s| q[s] as u64 * if i.strata[s].multi > 0 { 2 } else { 1 })
        .collect();
    let tie_w = |a: usize, b: usize| w[b].cmp(&w[a]).then(a.cmp(&b));
    let mut m = hamilton(i.comp.multi, &w, &tie_w);
    let mut overflow = 0usize;
    for s in 0..sn {
        if m[s] > q[s] {
            overflow += m[s] - q[s];
            m[s] = q[s];
        }
    }
    if overflow > 0 {
        let mut cand: Vec<usize> = (0..sn).filter(|&s| m[s] < q[s]).collect();
        cand.sort_by(|&a, &b| tie_w(a, b));
        while overflow > 0 {
            let mut progressed = false;
            for &s in &cand {
                if overflow == 0 {
                    break;
                }
                if m[s] < q[s] {
                    m[s] += 1;
                    overflow -= 1;
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
    }
    let mut drafts: Vec<Draft> = Vec::with_capacity(goal);
    for s in 0..sn {
        for k in 0..q[s] {
            let kind = if k < q[s] - m[s] {
                Kind::Single
            } else {
                Kind::Multi
            };
            drafts.push(Draft {
                stratum: s,
                kind,
                opts: 0,
                correct: Vec::new(),
                diff: None,
                seed: String::new(),
            });
        }
    }

    // P4: option counts.
    for d in drafts.iter_mut() {
        let of_kind = |it: &&SourceItem| it.kind == d.kind;
        let local = mode(
            items
                .iter()
                .filter(of_kind)
                .filter(|it| it.stratum == d.stratum)
                .map(|it| it.opts),
        );
        let global = mode(items.iter().filter(of_kind).map(|it| it.opts));
        let (default, lo) = match d.kind {
            Kind::Single => (4, 3),
            Kind::Multi => (5, 4),
        };
        d.opts = local.or(global).unwrap_or(default).clamp(lo, 6);
    }

    // P5: correct counts of the multiple-choice slots.
    let multi_idx: Vec<usize> = (0..drafts.len())
        .filter(|&k| drafts[k].kind == Kind::Multi)
        .collect();
    let mut counts: BTreeMap<u8, usize> = BTreeMap::new();
    for it in items.iter().filter(|it| it.kind == Kind::Multi) {
        *counts.entry(it.right.clamp(2, 5)).or_insert(0) += 1;
    }
    let mut cc: Vec<usize> = vec![2; drafts.len()];
    if !multi_idx.is_empty() && !counts.is_empty() {
        let values: Vec<u8> = counts.keys().copied().collect();
        let weights: Vec<u64> = counts.values().map(|&c| c as u64).collect();
        let share = hamilton(multi_idx.len(), &weights, &|a, b| a.cmp(&b));
        let mut expanded: Vec<u8> = Vec::new();
        for (v, n) in values.iter().zip(share) {
            expanded.extend(std::iter::repeat_n(*v, n));
        }
        let mut order = multi_idx.clone();
        rng.shuffle(&mut order);
        for (k, &slot) in order.iter().enumerate() {
            cc[slot] = expanded[k] as usize;
        }
    }
    for &k in &multi_idx {
        let top = (drafts[k].opts as usize).saturating_sub(1).max(2);
        cc[k] = cc[k].clamp(2, top);
    }

    // P6: answer positions, balanced over the whole bank.
    let mut order: Vec<usize> = (0..drafts.len()).collect();
    rng.shuffle(&mut order);
    let mut used = [0usize; 6];
    for &k in &order {
        let opts = drafts[k].opts as usize;
        let c = if drafts[k].kind == Kind::Single {
            1
        } else {
            cc[k]
        };
        let mut chosen: Vec<u8> = Vec::new();
        for _ in 0..c {
            let free: Vec<usize> = (0..opts)
                .filter(|p| !chosen.contains(&(*p as u8)))
                .collect();
            let least = free.iter().map(|&p| used[p]).min().unwrap_or(0);
            let tied: Vec<usize> = free.iter().copied().filter(|&p| used[p] == least).collect();
            let at = if tied.len() > 1 {
                rng.below(tied.len())
            } else {
                0
            };
            chosen.push(tied[at] as u8);
        }
        chosen.sort_unstable();
        for &p in &chosen {
            used[p as usize] += 1;
        }
        drafts[k].correct = chosen;
    }

    // P7: difficulty mirror per sheet.
    for s in 0..sn {
        let known: Vec<Diff> = items
            .iter()
            .filter(|it| it.stratum == s)
            .filter_map(|it| it.diff)
            .collect();
        if known.len() < 3 {
            continue;
        }
        let mut idx: Vec<usize> = (0..drafts.len())
            .filter(|&k| drafts[k].stratum == s)
            .collect();
        if idx.is_empty() {
            continue;
        }
        let count = |d: Diff| known.iter().filter(|&&x| x == d).count() as u64;
        let weights = [count(Diff::Easy), count(Diff::Mid), count(Diff::Hard)];
        // Ties: Mid, Easy, Hard (indexes 1, 0, 2).
        let rank = |a: usize| match a {
            1 => 0,
            0 => 1,
            _ => 2,
        };
        let share = hamilton(idx.len(), &weights, &|a, b| rank(a).cmp(&rank(b)));
        let mut labels: Vec<Diff> = Vec::new();
        for (d, n) in [Diff::Easy, Diff::Mid, Diff::Hard].iter().zip(share) {
            labels.extend(std::iter::repeat_n(*d, n));
        }
        rng.shuffle(&mut idx);
        for (k, &slot) in idx.iter().enumerate() {
            drafts[slot].diff = Some(labels[k]);
        }
    }

    // P8: seeds (no copy, wide coverage).
    let mut seed_used: HashMap<String, usize> = HashMap::new();
    let no_exclusions = HashSet::new();
    for d in drafts.iter_mut() {
        let id = pick_seed(
            d.stratum,
            d.kind,
            d.diff,
            &items,
            &seed_used,
            &no_exclusions,
            &mut rng,
        )
        .ok_or(PlanError::NoSource)?;
        *seed_used.entry(id.clone()).or_insert(0) += 1;
        d.seed = id;
    }

    // P9: final order, sheet by sheet; inside a sheet the slots are shuffled.
    let mut final_order: Vec<usize> = Vec::with_capacity(drafts.len());
    for s in 0..sn {
        let mut idx: Vec<usize> = (0..drafts.len())
            .filter(|&k| drafts[k].stratum == s)
            .collect();
        rng.shuffle(&mut idx);
        final_order.extend(idx);
    }
    let slots: Vec<Slot> = final_order
        .iter()
        .enumerate()
        .map(|(pos, &k)| {
            let d = &drafts[k];
            Slot {
                no: pos + 1,
                stratum: d.stratum,
                kind: d.kind,
                opts: d.opts,
                correct: d.correct.clone(),
                seed: d.seed.clone(),
                diff: d.diff,
            }
        })
        .collect();

    // P10: batches.
    let per_call = i.per_call.max(1);
    let batches: Vec<Vec<usize>> = (0..slots.len())
        .collect::<Vec<_>>()
        .chunks(per_call)
        .map(|c| c.to_vec())
        .collect();
    Ok(Plan {
        slots,
        batches,
        per_call,
    })
}

/// P8 again for the slots whose seed is in `missing` (every id missing so far, accumulated by the caller), never
/// choosing a missing id. When a slot's sheet has no present item left, the least-used present item of any sheet is
/// taken (ties: seeded random). A slot is never removed: it keeps its sheet, kind, option count, answer positions,
/// difficulty and number, so the goal never shrinks. Returns the new seed ids to fetch, or `Err(NoSource)` when no
/// source item is present at all.
pub fn replace_seeds(
    plan: &mut Plan,
    i: &PlanInput,
    missing: &[String],
) -> Result<Vec<String>, PlanError> {
    let gone: HashSet<String> = missing.iter().cloned().collect();
    // The same items as `plan` uses: an item whose sheet is out of range is ignored.
    let items: Vec<SourceItem> = i
        .items
        .iter()
        .filter(|it| it.stratum < i.strata.len())
        .cloned()
        .collect();
    if !items.iter().any(|it| !gone.contains(&it.id)) {
        return Err(PlanError::NoSource);
    }
    let mut used: HashMap<String, usize> = HashMap::new();
    for s in plan.slots.iter().filter(|s| !gone.contains(&s.seed)) {
        *used.entry(s.seed.clone()).or_insert(0) += 1;
    }
    // A different stream per round, so a second replacement does not repeat the first draws.
    let mut rng =
        Rng::new(i.seed ^ 0xA5A5_5A5A_DEAD_BEEF ^ (gone.len() as u64).wrapping_mul(0x9E37_79B9));
    let mut fresh: Vec<String> = Vec::new();
    for slot in plan.slots.iter_mut().filter(|s| gone.contains(&s.seed)) {
        let id = pick_seed(
            slot.stratum,
            slot.kind,
            slot.diff,
            &items,
            &used,
            &gone,
            &mut rng,
        )
        .ok_or(PlanError::NoSource)?;
        *used.entry(id.clone()).or_insert(0) += 1;
        if !fresh.contains(&id) {
            fresh.push(id.clone());
        }
        slot.seed = id;
    }
    Ok(fresh)
}

/// Questions per model call: `clamp(min(call_chars, ceiling * 2 / 3) / EST_CHARS_PER_QUESTION, 1, MAX_PER_CALL)`.
pub fn per_call(call_chars: usize, token_ceiling: Option<usize>) -> usize {
    let room = match token_ceiling {
        Some(t) => call_chars.min(t * 2 / 3),
        None => call_chars,
    };
    (room / EST_CHARS_PER_QUESTION).clamp(1, MAX_PER_CALL)
}

/// Output tokens for `call_chars` characters of Chinese / Japanese text: `limits::chars_to_tokens(chars)`.
pub fn call_tokens(call_chars: usize) -> usize {
    usize::try_from(crate::limits::chars_to_tokens(
        i64::try_from(call_chars).unwrap_or(i64::MAX),
    ))
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen::testkit::*;

    fn run_plan(single: usize, multi: usize, seed: u64) -> Plan {
        let items = items3();
        let strata = strata3();
        plan(&PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition { single, multi },
            per_call: 5,
            seed,
        })
        .unwrap()
    }

    fn per_stratum(p: &Plan) -> Vec<usize> {
        let mut v = vec![0; 3];
        for s in &p.slots {
            v[s.stratum] += 1;
        }
        v
    }

    #[test]
    fn per_call_vectors() {
        assert_eq!(per_call(8000, None), 5);
        assert_eq!(per_call(8000, Some(4096)), 1);
        assert_eq!(per_call(8000, Some(8192)), 3);
        assert_eq!(per_call(1000, None), 1);
        assert_eq!(per_call(32000, None), 5);
        assert_eq!(per_call(4800, None), 3);
    }

    #[test]
    fn call_tokens_matches_the_zh_factor() {
        assert_eq!(call_tokens(8000), 12000);
        assert_eq!(call_tokens(1000), 1500);
        assert_eq!(call_tokens(1), 2);
    }

    #[test]
    fn bad_composition_and_no_source() {
        let items = items3();
        let strata = strata3();
        let mk = |single, multi, items: &[SourceItem]| {
            plan(&PlanInput {
                items,
                strata: &strata,
                comp: Composition { single, multi },
                per_call: 5,
                seed: 1,
            })
        };
        assert_eq!(mk(0, 0, &items).unwrap_err(), PlanError::BadComposition);
        assert_eq!(mk(100, 1, &items).unwrap_err(), PlanError::BadComposition);
        assert_eq!(mk(14, 6, &[]).unwrap_err(), PlanError::NoSource);
        assert!(mk(100, 0, &items).is_ok());
        assert!(mk(0, 5, &items).is_ok());
    }

    #[test]
    fn quotas_follow_sheet_sizes_with_at_least_one_each() {
        let p = run_plan(14, 6, 11);
        // Sizes 20 / 10 / 10 over 20 questions: 10 / 5 / 5.
        assert_eq!(per_stratum(&p), vec![10, 5, 5]);
        let tiny = run_plan(2, 1, 11);
        assert_eq!(per_stratum(&tiny).iter().sum::<usize>(), 3);
        assert!(per_stratum(&tiny).iter().all(|&n| n >= 1));
    }

    #[test]
    fn small_sheet_still_gets_one_question() {
        let mut strata = strata3();
        strata[1].n = 1;
        strata[2].n = 1;
        let items = items3();
        let p = plan(&PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 14,
                multi: 6,
            },
            per_call: 5,
            seed: 3,
        })
        .unwrap();
        let ps = per_stratum(&p);
        assert!(ps[1] >= 1 && ps[2] >= 1);
        assert_eq!(ps.iter().sum::<usize>(), 20);
    }

    #[test]
    fn more_sheets_than_questions_pick_the_larger_sheets() {
        let strata: Vec<Stratum> = (0..6)
            .map(|k| Stratum {
                bank: 1,
                sheet: format!("S{k}"),
                n: 10 + k,
                multi: 0,
            })
            .collect();
        let items: Vec<SourceItem> = (0..6)
            .map(|k| SourceItem {
                id: format!("i{k}"),
                stratum: k,
                kind: Kind::Single,
                opts: 4,
                right: 1,
                diff: None,
                stem: tri(k, 10),
            })
            .collect();
        let p = plan(&PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 3,
                multi: 0,
            },
            per_call: 5,
            seed: 9,
        })
        .unwrap();
        let mut used: Vec<usize> = p.slots.iter().map(|s| s.stratum).collect();
        used.sort();
        assert_eq!(used, vec![3, 4, 5]);
    }

    #[test]
    fn equal_sized_sheets_are_picked_at_random() {
        let strata: Vec<Stratum> = (0..8)
            .map(|k| Stratum {
                bank: 1,
                sheet: format!("S{k}"),
                n: 10,
                multi: 0,
            })
            .collect();
        let items: Vec<SourceItem> = (0..8)
            .map(|k| SourceItem {
                id: format!("i{k}"),
                stratum: k,
                kind: Kind::Single,
                opts: 4,
                right: 1,
                diff: None,
                stem: tri(k, 10),
            })
            .collect();
        let mut seen = HashSet::new();
        for seed in 0..40u64 {
            let p = plan(&PlanInput {
                items: &items,
                strata: &strata,
                comp: Composition {
                    single: 2,
                    multi: 0,
                },
                per_call: 5,
                seed,
            })
            .unwrap();
            let mut u: Vec<usize> = p.slots.iter().map(|s| s.stratum).collect();
            u.sort();
            seen.insert(u);
        }
        assert!(seen.len() > 3, "different seeds must pick different sheets");
    }

    #[test]
    fn composition_is_exact_with_and_without_multi_sources() {
        for (single, multi) in [(14, 6), (20, 0), (0, 20), (1, 1)] {
            let p = run_plan(single, multi, 5);
            let m = p.slots.iter().filter(|s| s.kind == Kind::Multi).count();
            assert_eq!((p.slots.len() - m, m), (single, multi));
        }
        // Sheet 1 has no multiple-choice source: the weights prefer the other sheets (the Hamilton remainders may
        // still give it one), so it gets fewer multis than the biggest sheet that has multiple-choice sources.
        let p = run_plan(14, 6, 5);
        let multis = |st: usize| {
            p.slots
                .iter()
                .filter(|s| s.stratum == st && s.kind == Kind::Multi)
                .count()
        };
        assert!(
            multis(1) <= 1 && multis(0) >= 3,
            "{} {}",
            multis(0),
            multis(1)
        );
    }

    #[test]
    fn multi_overflow_goes_to_sheets_with_room() {
        let p = run_plan(0, 20, 5);
        assert!(p.slots.iter().all(|s| s.kind == Kind::Multi));
        assert_eq!(per_stratum(&p), vec![10, 5, 5]);
    }

    #[test]
    fn option_and_correct_counts_are_in_range() {
        let p = run_plan(14, 6, 21);
        for s in &p.slots {
            match s.kind {
                Kind::Single => {
                    assert_eq!(s.opts, 4);
                    assert_eq!(s.correct.len(), 1);
                }
                Kind::Multi => {
                    assert_eq!(s.opts, 5);
                    assert!((2..=4).contains(&s.correct.len()));
                }
            }
            assert!(s.correct.windows(2).all(|w| w[0] < w[1]));
            assert!(s.correct.iter().all(|&c| c < s.opts));
        }
    }

    #[test]
    fn no_multi_sources_means_two_correct_each() {
        let strata = vec![Stratum {
            bank: 1,
            sheet: "Only".into(),
            n: 10,
            multi: 0,
        }];
        let items: Vec<SourceItem> = (0..5)
            .map(|k| SourceItem {
                id: format!("i{k}"),
                stratum: 0,
                kind: Kind::Single,
                opts: 4,
                right: 1,
                diff: None,
                stem: tri(k, 10),
            })
            .collect();
        let p = plan(&PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 2,
                multi: 3,
            },
            per_call: 5,
            seed: 2,
        })
        .unwrap();
        for s in p.slots.iter().filter(|s| s.kind == Kind::Multi) {
            assert_eq!(s.correct.len(), 2);
            assert_eq!(s.opts, 5);
        }
    }

    #[test]
    fn twenty_four_option_singles_use_each_letter_five_times() {
        let strata = vec![Stratum {
            bank: 1,
            sheet: "Only".into(),
            n: 40,
            multi: 0,
        }];
        let items: Vec<SourceItem> = (0..30)
            .map(|k| SourceItem {
                id: format!("i{k}"),
                stratum: 0,
                kind: Kind::Single,
                opts: 4,
                right: 1,
                diff: None,
                stem: tri(k, 10),
            })
            .collect();
        for seed in [1u64, 2, 3] {
            let p = plan(&PlanInput {
                items: &items,
                strata: &strata,
                comp: Composition {
                    single: 20,
                    multi: 0,
                },
                per_call: 5,
                seed,
            })
            .unwrap();
            let mut used = [0usize; 4];
            for s in &p.slots {
                used[s.correct[0] as usize] += 1;
            }
            assert_eq!(used, [5, 5, 5, 5]);
        }
    }

    #[test]
    fn positions_are_balanced_for_mixed_slots() {
        for seed in 0..25u64 {
            let p = run_plan(14, 6, seed);
            let min_opts = p.slots.iter().map(|s| s.opts).min().unwrap() as usize;
            let mut used = vec![0usize; 6];
            for s in &p.slots {
                for &c in &s.correct {
                    used[c as usize] += 1;
                }
            }
            let window = &used[..min_opts];
            let spread = window.iter().max().unwrap() - window.iter().min().unwrap();
            assert!(spread <= 1, "seed {seed}: {used:?}");
        }
    }

    #[test]
    fn difficulty_mirrors_the_source_per_sheet() {
        let p = run_plan(14, 6, 8);
        for st in 0..3 {
            let mine: Vec<&Slot> = p.slots.iter().filter(|s| s.stratum == st).collect();
            assert!(mine.iter().all(|s| s.diff.is_some()));
            let easy = mine.iter().filter(|s| s.diff == Some(Diff::Easy)).count();
            let mid = mine.iter().filter(|s| s.diff == Some(Diff::Mid)).count();
            let hard = mine.iter().filter(|s| s.diff == Some(Diff::Hard)).count();
            // The sample has an even split of the three labels.
            let n = mine.len();
            assert!(
                easy.abs_diff(n / 3) <= 1 && mid.abs_diff(n / 3) <= 1 && hard.abs_diff(n / 3) <= 1
            );
        }
    }

    #[test]
    fn fewer_than_three_known_difficulties_means_none() {
        let mut items = items3();
        for it in items.iter_mut() {
            it.diff = None;
        }
        items[0].diff = Some(Diff::Easy);
        let strata = strata3();
        let p = plan(&PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 14,
                multi: 6,
            },
            per_call: 5,
            seed: 4,
        })
        .unwrap();
        assert!(p.slots.iter().all(|s| s.diff.is_none()));
    }

    #[test]
    fn seeds_are_distinct_until_a_sheet_is_exhausted() {
        let p = run_plan(14, 6, 13);
        // Sheet 0: 12 items for 10 slots; sheet 1: 8 for 5; sheet 2: 8 for 5.
        let mut seeds: Vec<&str> = p.slots.iter().map(|s| s.seed.as_str()).collect();
        seeds.sort();
        seeds.dedup();
        assert_eq!(seeds.len(), 20);
        for s in &p.slots {
            assert!(s.seed.starts_with(&format!("s{}x", s.stratum)));
        }
    }

    #[test]
    fn plan_is_deterministic_and_seed_sensitive() {
        assert_eq!(run_plan(14, 6, 99), run_plan(14, 6, 99));
        assert_ne!(run_plan(14, 6, 99), run_plan(14, 6, 100));
    }

    #[test]
    fn slots_are_numbered_and_grouped_by_sheet() {
        let p = run_plan(14, 6, 31);
        for (k, s) in p.slots.iter().enumerate() {
            assert_eq!(s.no, k + 1);
        }
        assert!(p.slots.windows(2).all(|w| w[0].stratum <= w[1].stratum));
        // Multis are not always last inside a sheet over many seeds.
        let mut multi_first = false;
        for seed in 0..30u64 {
            let q = run_plan(14, 6, seed);
            if q.slots.windows(2).any(|w| {
                w[0].stratum == w[1].stratum
                    && w[0].kind == Kind::Multi
                    && w[1].kind == Kind::Single
            }) {
                multi_first = true;
            }
        }
        assert!(multi_first);
    }

    #[test]
    fn batches_are_consecutive_chunks() {
        let p = run_plan(14, 6, 1);
        assert_eq!(p.batches.len(), 4);
        assert!(p.batches.iter().all(|b| b.len() == 5));
        let flat: Vec<usize> = p.batches.iter().flatten().copied().collect();
        assert_eq!(flat, (0..20).collect::<Vec<_>>());
        let items = items3();
        let strata = strata3();
        let q = plan(&PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 14,
                multi: 6,
            },
            per_call: 3,
            seed: 1,
        })
        .unwrap();
        assert_eq!(q.batches.len(), 7);
        assert_eq!(q.batches.last().unwrap().len(), 2);
        assert_eq!(q.per_call, 3);
    }

    #[test]
    fn replace_seeds_keeps_every_slot() {
        let items = items3();
        let strata = strata3();
        let input = PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 14,
                multi: 6,
            },
            per_call: 5,
            seed: 6,
        };
        let mut p = plan(&input).unwrap();
        let before = p.clone();
        let missing: Vec<String> = p.slots.iter().take(4).map(|s| s.seed.clone()).collect();
        let fresh = replace_seeds(&mut p, &input, &missing).unwrap();
        assert!(!fresh.is_empty());
        assert_eq!(p.slots.len(), 20);
        for (a, b) in p.slots.iter().zip(&before.slots) {
            assert_eq!(
                (a.no, a.stratum, a.kind, a.opts, &a.correct, a.diff),
                (b.no, b.stratum, b.kind, b.opts, &b.correct, b.diff)
            );
            assert!(!missing.contains(&a.seed));
        }
        assert!(fresh.iter().all(|f| !missing.contains(f)));
    }

    #[test]
    fn replace_seeds_falls_back_across_sheets() {
        let items = items3();
        let strata = strata3();
        let input = PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 14,
                multi: 6,
            },
            per_call: 5,
            seed: 6,
        };
        let mut p = plan(&input).unwrap();
        // Every item of sheet 1 is missing: its five slots take seeds of other sheets.
        let missing: Vec<String> = items
            .iter()
            .filter(|it| it.stratum == 1)
            .map(|it| it.id.clone())
            .collect();
        replace_seeds(&mut p, &input, &missing).unwrap();
        assert_eq!(p.slots.len(), 20);
        for s in p.slots.iter().filter(|s| s.stratum == 1) {
            assert!(!s.seed.starts_with("s1x"));
        }
    }

    #[test]
    fn replace_seeds_fails_only_when_nothing_is_left() {
        let items = items3();
        let strata = strata3();
        let input = PlanInput {
            items: &items,
            strata: &strata,
            comp: Composition {
                single: 14,
                multi: 6,
            },
            per_call: 5,
            seed: 6,
        };
        let mut p = plan(&input).unwrap();
        let all: Vec<String> = items.iter().map(|it| it.id.clone()).collect();
        assert_eq!(
            replace_seeds(&mut p, &input, &all).unwrap_err(),
            PlanError::NoSource
        );
        assert_eq!(p.slots.len(), 20);
        // Nothing missing: nothing to do.
        assert!(replace_seeds(&mut p, &input, &[]).unwrap().is_empty());
    }

    #[test]
    fn hamilton_gives_the_exact_total() {
        let tie = |a: usize, b: usize| a.cmp(&b);
        assert_eq!(hamilton(20, &[20, 10, 10], &tie), vec![10, 5, 5]);
        assert_eq!(hamilton(7, &[1, 1, 1], &tie), vec![3, 2, 2]);
        assert_eq!(hamilton(5, &[0, 0], &tie), vec![0, 0]);
        assert_eq!(hamilton(3, &[5, 1], &tie).iter().sum::<usize>(), 3);
    }
}
