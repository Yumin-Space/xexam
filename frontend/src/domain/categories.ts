// Bank categories (v3.1): grouping the bank list. Pure functions, shared by the banks page, the Home picker and the study
// records filter. Order rules (contract D18): categories by (sort, id); banks inside a group by ((sort = 0), sort, id), so
// banks that were never placed (sort 0) come after the placed ones, in id order.
import type { BankCategory, BankSummary } from '../types/api.ts';

export interface BankGroup<B> { category: BankCategory | null; banks: B[] }

type Placed = Pick<BankSummary, 'id' | 'categoryId' | 'sort'>;

const byPlace = (a: Placed, b: Placed): number =>
  (a.sort === 0 ? 1 : 0) - (b.sort === 0 ? 1 : 0) || a.sort - b.sort || a.id - b.id;

/** Categories in (sort, id) order, each with its banks in ((sort = 0), sort, id) order, then "Uncategorised" (only when not
 *  empty, same bank order). No category at all gives one group (category null) holding every bank in the given order.
 *  A bank whose category is not in the list counts as uncategorised. */
export function groupBanks<B extends Placed>(banks: readonly B[],
  categories: readonly BankCategory[]): { hasCategories: boolean; groups: BankGroup<B>[] } {
  if (!categories.length) return { hasCategories: false, groups: [{ category: null, banks: [...banks] }] };
  const cats = [...categories].sort((a, b) => a.sort - b.sort || a.id - b.id);
  const known = new Set(cats.map((c) => c.id));
  const groups: BankGroup<B>[] = cats.map((category) => ({
    category,
    banks: banks.filter((b) => b.categoryId === category.id).sort(byPlace),
  }));
  const rest = banks.filter((b) => b.categoryId == null || !known.has(b.categoryId)).sort(byPlace);
  if (rest.length) groups.push({ category: null, banks: rest });
  return { hasCategories: true, groups };
}

/** Tri-state of a category checkbox from the keys it covers and the selected keys (no keys = 'none') */
export function triState(keys: readonly string[], selected: ReadonlySet<string>): 'all' | 'some' | 'none' {
  let on = 0;
  for (const k of keys) if (selected.has(k)) on++;
  if (on === 0) return 'none';
  return on === keys.length ? 'all' : 'some';
}

/** The ids with `id` moved one place up (-1) or down (+1); unchanged at the ends or when `id` is not in the list */
export function moveInList(ids: readonly number[], id: number, delta: -1 | 1): number[] {
  const out = [...ids];
  const i = out.indexOf(id);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= out.length) return out;
  [out[i], out[j]] = [out[j]!, out[i]!];
  return out;
}
