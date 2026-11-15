// The rows behind the two bar charts of AI settings > AI 用量 (v1.1, A-OWNER-39 / 40): the "other" bar that makes each chart
// add up to the total, the K / M number format and the calls / tokens metric. Pure: no DOM, no i18n calls (runs under `node --test`).

export interface Counts {
  calls: number;
  tokensIn: number;
  tokensOut: number;
  errors: number;
  /** Cache-read / cache-write input tokens (A-OWNER-74); missing = the provider reported nothing */
  tokensCached?: number;
  tokensCacheWrite?: number;
}

/** One bar: a label, the counts and whether it is the "other" bar */
export interface UsageRow extends Counts {
  key: string;
  label: string;
  other?: boolean;
}

export type UsageMetric = 'calls' | 'tokens';

const ZERO: Counts = { calls: 0, tokensIn: 0, tokensOut: 0, errors: 0 };

/** The two cache figures of a sum: present only when one of the parts has them */
const cacheSum = (a: Counts, b: Counts): Pick<Counts, 'tokensCached' | 'tokensCacheWrite'> => ({
  ...(a.tokensCached != null || b.tokensCached != null ? { tokensCached: (a.tokensCached ?? 0) + (b.tokensCached ?? 0) } : {}),
  ...(a.tokensCacheWrite != null || b.tokensCacheWrite != null ? { tokensCacheWrite: (a.tokensCacheWrite ?? 0) + (b.tokensCacheWrite ?? 0) } : {}),
});

export const add = (a: Counts, b: Counts): Counts => ({
  calls: a.calls + b.calls, tokensIn: a.tokensIn + b.tokensIn, tokensOut: a.tokensOut + b.tokensOut, errors: a.errors + b.errors, ...cacheSum(a, b),
});

const nonNeg = (c: Counts): Counts => ({
  calls: Math.max(0, c.calls), tokensIn: Math.max(0, c.tokensIn), tokensOut: Math.max(0, c.tokensOut), errors: Math.max(0, c.errors),
  ...(c.tokensCached != null ? { tokensCached: Math.max(0, c.tokensCached) } : {}),
  ...(c.tokensCacheWrite != null ? { tokensCacheWrite: Math.max(0, c.tokensCacheWrite) } : {}),
});

const used = (c: Counts): boolean => c.calls > 0 || c.tokensIn > 0 || c.tokensOut > 0 || c.errors > 0;

/**
 * Compact number, the same in every language (A-OWNER-59): K from 1 000, M from 1 000 000 (1.2K, 48K, 3.4M); one decimal at
 * most, no trailing ".0"; below 1 000 the plain digits. Never 万.
 */
export function formatBig(n: number): string {
  const v = Math.max(0, Math.round(n));
  const one = (x: number): string => String(Math.round(x * 10) / 10);
  if (v >= 1_000_000 || Math.round(v / 100) / 10 >= 1000) return `${one(v / 1_000_000)}M`;
  if (v >= 1000) return `${one(v / 1000)}K`;
  return String(v);
}

/** "write/read" pair of the cache figures in one format (A-OWNER-84: write first, then read) */
export const cachePair = (write: number, read: number, fmt: (n: number) => string = formatBig): string => `${fmt(write)}/${fmt(read)}`;

/**
 * A-OWNER-84: the figures of the live token text. `io` is "input/output", `cache` is "write/read" and null while both are 0
 * (row 2 is then not drawn). Missing cache figures count as 0.
 */
export function tokenFigures(u: { tokensIn: number; tokensOut: number; tokensCached?: number; tokensCacheWrite?: number }): { io: string; cache: string | null } {
  const w = Math.max(0, u.tokensCacheWrite ?? 0);
  const r = Math.max(0, u.tokensCached ?? 0);
  return { io: cachePair(u.tokensIn, u.tokensOut), cache: w > 0 || r > 0 ? cachePair(w, r) : null };
}

/** The bar value of a row in a metric (tokens: input + output) */
export const valueOf = (c: Counts, m: UsageMetric): number => (m === 'calls' ? c.calls : c.tokensIn + c.tokensOut);

/**
 * By model: the rows with a model are bars; the calls without a model (model lists) are not a model, they join the "other" bar.
 * Rows of the same model are summed. The "other" bar is last and only present when something was counted there.
 */
export function modelRows(rows: ReadonlyArray<Counts & { model: string }>, otherLabel: string): UsageRow[] {
  const named = new Map<string, Counts>();
  let other: Counts = ZERO;
  for (const r of rows) {
    if (r.model) named.set(r.model, add(named.get(r.model) ?? ZERO, r));
    else other = add(other, r);
  }
  const out: UsageRow[] = [...named].map(([model, c]) => ({ key: model, label: model, ...c }));
  if (used(other)) out.push({ key: '', label: otherLabel, other: true, ...other });
  return out;
}

/**
 * By feature: one bar per listed feature (zero ones too), and an "other" bar of what the total holds beyond them (model lists,
 * connection tests, ...), so the chart adds up to `totals`. Never negative.
 */
export function purposeRows<K extends string>(
  keys: readonly K[], rows: ReadonlyArray<Counts & { purpose: string }>, totals: Counts, labelOf: (k: K) => string, otherLabel: string,
): UsageRow[] {
  const out: UsageRow[] = keys.map((k) => ({
    key: k, label: labelOf(k), ...rows.filter((r) => r.purpose === k).reduce<Counts>((a, r) => add(a, r), ZERO),
  }));
  const sum = out.reduce<Counts>((a, r) => add(a, r), ZERO);
  const rest = nonNeg({
    calls: totals.calls - sum.calls, tokensIn: totals.tokensIn - sum.tokensIn, tokensOut: totals.tokensOut - sum.tokensOut, errors: totals.errors - sum.errors,
    ...(totals.tokensCached != null ? { tokensCached: totals.tokensCached - (sum.tokensCached ?? 0) } : {}),
    ...(totals.tokensCacheWrite != null ? { tokensCacheWrite: totals.tokensCacheWrite - (sum.tokensCacheWrite ?? 0) } : {}),
  });
  if (used(rest)) out.push({ key: '', label: otherLabel, other: true, ...rest });
  return out;
}

export type UsageSort = 'amount' | 'name';

/** Name groups (owner, v1.3): 0 = starts with a digit, 1 = starts with a Latin letter, 2 = any other script */
const nameGroup = (s: string): number => (/^[0-9]/.test(s) ? 0 : /^[A-Za-z]/.test(s) ? 1 : 2);

/**
 * The order of the bars (owner, v1.3): 'amount' = high to low by the metric shown; 'name' = names starting with a digit first,
 * then names starting with a Latin letter (A-Z by the first letter, case-insensitive, then the rest with natural numbers:
 * "m2" before "m10"), then every other script (Chinese, Japanese, ...) among themselves with the UI-locale collator. The same
 * rule in every UI language. Ties keep the incoming order (stable). In name order the "other" bar stays last; in amount order
 * it ranks by its value like any bar. Returns a new array.
 */
export function sortRows(rows: readonly UsageRow[], sort: UsageSort, metric: UsageMetric, locale?: string): UsageRow[] {
  const coll = new Intl.Collator(locale, { numeric: true, sensitivity: 'base' });
  const latin = new Intl.Collator('en', { numeric: true, sensitivity: 'base' });
  const idx = new Map(rows.map((r, i) => [r, i]));
  const byName = (a: string, b: string): number => {
    const ga = nameGroup(a);
    const gb = nameGroup(b);
    if (ga !== gb) return ga - gb;
    if (ga === 1) return a[0]!.toLowerCase().localeCompare(b[0]!.toLowerCase(), 'en') || latin.compare(a.slice(1), b.slice(1));
    return (ga === 0 ? latin : coll).compare(a, b);
  };
  const cmp = sort === 'name'
    ? (a: UsageRow, b: UsageRow): number => Number(!!a.other) - Number(!!b.other) || byName(a.label, b.label)
    : (a: UsageRow, b: UsageRow): number => valueOf(b, metric) - valueOf(a, metric);
  return [...rows].sort((a, b) => cmp(a, b) || idx.get(a)! - idx.get(b)!);
}
