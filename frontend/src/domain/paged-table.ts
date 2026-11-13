// Search, sort and paging for the long admin tables (v1.1 L-54, A-OWNER-54): the AI usage "by user" table and the
// AI+ users list. Pure functions; the DOM part is components/paged-table.ts. Order of work: search, then sort, then page.

export const PAGE_SIZE = 10;

export type SortDir = 'asc' | 'desc';

export interface TableState {
  query: string;
  key: string;
  dir: SortDir;
  /** 1-based */
  page: number;
}

export interface PagedResult<T> {
  /** The rows of the current page */
  items: T[];
  /** Rows after the search (before paging) */
  total: number;
  pages: number;
  /** The page actually shown (clamped) */
  page: number;
  from: number;
  to: number;
}

/** A search matches when every blank-separated word is inside the name (case-insensitive) */
export function matchesQuery(name: string, query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = name.toLowerCase();
  return words.every((w) => hay.includes(w));
}

/** Text columns start ascending (A to Z), number and time columns descending (largest first) */
export function firstDir(textColumn: boolean): SortDir {
  return textColumn ? 'asc' : 'desc';
}

/** A header click: the same column reverses, another column starts in its first direction */
export function clickSort(state: TableState, key: string, textColumn: boolean): TableState {
  if (state.key === key) return { ...state, dir: state.dir === 'asc' ? 'desc' : 'asc', page: 1 };
  return { ...state, key, dir: firstDir(textColumn), page: 1 };
}

/** Typing a search returns to page 1 */
export function withQuery(state: TableState, query: string): TableState {
  return { ...state, query, page: 1 };
}

export function pageTable<T>(rows: readonly T[], state: TableState, opts: {
  name: (r: T) => string;
  /** One comparator per sortable key (ascending); the name breaks ties */
  cmp: Record<string, (a: T, b: T) => number>;
  size?: number;
}): PagedResult<T> {
  const size = opts.size ?? PAGE_SIZE;
  const hit = rows.filter((r) => matchesQuery(opts.name(r), state.query));
  const base = opts.cmp[state.key];
  const sign = state.dir === 'asc' ? 1 : -1;
  const byName = opts.cmp.user ?? ((a: T, b: T) => opts.name(a).localeCompare(opts.name(b)));
  const sorted = base ? [...hit].sort((a, b) => sign * base(a, b) || byName(a, b)) : hit;
  const pages = Math.max(1, Math.ceil(sorted.length / size));
  const page = Math.min(Math.max(1, Math.floor(state.page) || 1), pages);
  const start = (page - 1) * size;
  const items = sorted.slice(start, start + size);
  return { items, total: sorted.length, pages, page, from: items.length ? start + 1 : 0, to: start + items.length };
}

/** The page buttons: 1 2 3 … last. `siblings` pages around the current one stay; gaps of one page are filled in. */
export function pageList(page: number, pages: number, siblings = 1): Array<number | 'gap'> {
  const keep = new Set<number>([1, pages]);
  for (let p = page - siblings; p <= page + siblings; p += 1) if (p >= 1 && p <= pages) keep.add(p);
  const nums = [...keep].sort((a, b) => a - b);
  const out: Array<number | 'gap'> = [];
  nums.forEach((n, i) => {
    const prev = nums[i - 1];
    if (prev !== undefined && n - prev === 2) out.push(prev + 1);
    else if (prev !== undefined && n - prev > 2) out.push('gap');
    out.push(n);
  });
  return out;
}
