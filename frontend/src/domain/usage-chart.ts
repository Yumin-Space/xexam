// The geometry of the three usage charts of AI settings > AI 用量 (v1.1, UC-1): nice axis ticks, the days / months of a
// period with the gaps filled by 0, a monotone cubic path (no overshoot, so a smooth line never dips below 0 between two
// points) and the thinning of crowded x labels. Pure: no DOM, no i18n (it runs under `node --test`); components draw it as
// inline SVG.

/** Rounds `v` up to 1, 2, 5 or 10 times a power of ten (at least 1) */
export function niceMax(v: number): number {
  if (!(v > 1)) return 1;
  const p = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 5, 10]) if (v <= m * p) return m * p;
  return 10 * p;
}

/** The step of an axis: 1, 2 or 5 times a power of ten that is at least `raw` (whole numbers only, never below 1) */
function niceStep(raw: number): number {
  if (!(raw > 1)) return 1;
  const p = 10 ** Math.floor(Math.log10(raw));
  for (const m of [1, 2, 5, 10]) if (raw <= m * p) return m * p;
  return 10 * p;
}

/**
 * The y ticks: `0` up to the first multiple of a nice step that holds `max`, at most `count + 1` of them, each a whole number
 * (counts of calls): a maximum of 3 gives 0, 1, 2, 3; 47 gives 0, 20, 40, 60.
 */
export function yTicks(max: number, count = 4): number[] {
  const m = Math.max(1, Math.ceil(max));
  const step = niceStep(m / count);
  const top = step * Math.ceil(m / step);
  const out: number[] = [];
  for (let v = 0; v <= top; v += step) out.push(v);
  return out;
}

/** The top of the y axis (the last tick) */
export const axisMax = (max: number, count = 4): number => yTicks(max, count).slice(-1)[0] ?? 1;

const DAY = 86_400_000;
const utc = (ymd: string): number => {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(ymd);
  return m ? Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : Number.NaN;
};
const ymdOf = (ms: number): string => new Date(ms).toISOString().slice(0, 10);

/** Every day from `from` to `to` (inclusive), as `YYYY-MM-DD`; empty when a bound is not a date or `to` is before `from` */
export function dayRange(from: string, to: string): string[] {
  const a = utc(from);
  const b = utc(to);
  if (Number.isNaN(a) || Number.isNaN(b) || b < a) return [];
  const out: string[] = [];
  for (let t = a; t <= b && out.length < 4000; t += DAY) out.push(ymdOf(t));
  return out;
}

/** Every month from the month of `from` to the month of `to` (inclusive), as `YYYY-MM` */
export function monthRange(from: string, to: string): string[] {
  const fm = /^(\d{4})-(\d{2})/.exec(from);
  const tm = /^(\d{4})-(\d{2})/.exec(to);
  if (!fm || !tm) return [];
  let y = Number(fm[1]);
  let m = Number(fm[2]);
  const ey = Number(tm[1]);
  const em = Number(tm[2]);
  const out: string[] = [];
  while ((y < ey || (y === ey && m <= em)) && out.length < 1200) {
    out.push(`${y}-${String(m).padStart(2, '0')}`);
    m += 1;
    if (m > 12) {
      m = 1;
      y += 1;
    }
  }
  return out;
}

export interface Point {
  period: string;
  calls: number;
}

/**
 * The x axis of the over-time line (UC-1): the periods of `from` .. `to` in order (full `YYYY-MM-DD` for unit day, `YYYY-MM`
 * for unit month), a period the server left out counts 0. A row outside the range is kept at its sorted place (the server's
 * word wins over the clock), so no number is ever dropped.
 */
export function fillPeriods(unit: 'day' | 'month', from: string, to: string, rows: ReadonlyArray<{ period: string; calls: number }>): Point[] {
  const have = new Map<string, number>();
  for (const r of rows) have.set(r.period, (have.get(r.period) ?? 0) + r.calls);
  const keys = new Set<string>(unit === 'day' ? dayRange(from, to) : monthRange(from, to));
  for (const k of have.keys()) keys.add(k);
  return [...keys].sort().map((period) => ({ period, calls: have.get(period) ?? 0 }));
}

/** How many labels share the room of one: label `i` is drawn when `i % step === 0` (`slot` = px per category, `labelWidth` in px) */
export function labelStep(slot: number, labelWidth: number, gap = 6): number {
  if (slot <= 0) return 1;
  return Math.max(1, Math.ceil((labelWidth + gap) / slot));
}

/** The estimated width of text at the chart's 11 px type (CJK characters are wider) */
export const textWidth = (s: string): number => [...s].reduce((w, c) => w + (c.charCodeAt(0) > 0x2e80 ? 11 : 6.2), 0);

/**
 * A bar label wrapped into at most `maxLines` lines that each fit `slot` px: breaks at spaces, and between characters of CJK
 * text (never inside a Latin word). Null when a Latin word is wider than the slot or the text needs more lines than allowed
 * (the caller then slants the labels instead). One layout for every language: the full text is only in the tip when null.
 */
export function wrapLabel(label: string, slot: number, maxLines = 3): string[] | null {
  const room = slot - 4;
  const lines: string[] = [];
  let cur = '';
  const isCjk = (c: string): boolean => c.charCodeAt(0) > 0x2e80;
  // tokens: a Latin word, a space, or one CJK character
  const tokens = label.match(/[^\s⺀-￿]+|\s+|[⺀-￿]/g) ?? [];
  for (const tok of tokens) {
    if (/^\s+$/.test(tok)) {
      if (cur !== '') cur += ' ';
      continue;
    }
    if (!isCjk(tok) && textWidth(tok) > room) return null;
    if (cur !== '' && textWidth(cur + tok) > room) {
      lines.push(cur.trimEnd());
      cur = '';
    }
    cur += tok;
  }
  if (cur.trim() !== '') lines.push(cur.trimEnd());
  return lines.length > 0 && lines.length <= maxLines ? lines : null;
}

/** A chart box this narrow (px) is a phone: the by-feature and by-model charts are horizontal there (A-OWNER-86) */
export const PHONE_CHART = 520;

/**
 * Whether the by-feature / by-model chart is drawn as horizontal rows: on a phone-wide box, or whenever the column chart could
 * not show every label flat (`wrapped` holds `wrapLabel` of each label; a null means that label does not fit its column).
 * No label is ever slanted, so there is no third layout.
 */
export const horizontalBars = (width: number, wrapped: ReadonlyArray<readonly string[] | null>): boolean =>
  width < PHONE_CHART || wrapped.some((l) => l == null);

/**
 * `s` cut with an ellipsis so that its width is at most `maxPx` (the full text is in the tip); at least one character. `width`
 * measures a text (default: the estimate `textWidth`; the chart passes a canvas measure of the real font).
 */
export function fitText(s: string, maxPx: number, width: (s: string) => number = textWidth): string {
  if (width(s) <= maxPx) return s;
  const a = [...s];
  let out = '';
  for (const c of a) {
    if (width(`${out}${c}…`) > maxPx) break;
    out += c;
  }
  return `${out === '' ? a[0]! : out}…`;
}

/**
 * The width (px) of the name column of the horizontal bars: the widest name, but at most `share` (40 percent) of the box and
 * never below `min`. Longer names are cut by `fitText`.
 */
export function nameColumn(widths: readonly number[], box: number, share = 0.4, min = 40): number {
  return Math.max(min, Math.min(Math.ceil(Math.max(0, ...widths)), Math.floor(box * share)));
}

/** The short form of a period for the x axis: `YYYY-MM-DD` becomes `MM-DD`; a month (`YYYY-MM`) already is short */
export function shortPeriod(period: string): string {
  const m = /^\d{4}-(\d{2}-\d{2})$/.exec(period);
  return m ? m[1]! : period;
}

/**
 * The indexes of the x labels to draw, counted from the newest one back (so the newest is always named): every `step`-th of
 * `n`. `fits(i)` may drop one that would leave the box (the oldest ones near the edge).
 */
export function newestIndexes(n: number, step: number, fits: (i: number) => boolean = () => true): number[] {
  const out: number[] = [];
  const s = Math.max(1, Math.floor(step));
  for (let i = n - 1; i >= 0; i -= s) if (i === n - 1 || fits(i)) out.push(i);
  return out.reverse();
}

export interface Bar {
  x: number;
  width: number;
  y: number;
  height: number;
  value: number;
}

/**
 * Vertical bars: `values` in the plot box (`left`, `top`, `width`, `height`), the axis maximum `max`. Each bar is at most
 * `maxBar` wide, centred in its slot; a zero value is a 0-high bar (the label still shows its 0).
 */
export function bars(values: readonly number[], box: { left: number; top: number; width: number; height: number }, max: number, maxBar = 36): Bar[] {
  const n = values.length;
  if (n === 0 || max <= 0) return [];
  const slot = box.width / n;
  const w = Math.min(maxBar, Math.max(2, slot * 0.62));
  return values.map((value, i) => {
    const height = (Math.max(0, value) / max) * box.height;
    return { x: box.left + slot * i + (slot - w) / 2, width: w, y: box.top + box.height - height, height, value };
  });
}

export interface HBar {
  /** the row's top */
  rowY: number;
  x: number;
  width: number;
  y: number;
  height: number;
  value: number;
}

/**
 * Horizontal bars, one row of `rowH` px each from `top`: the bar starts at `left` and grows right up to `width` px for the
 * maximum `max` (the room for the value at its end is the caller's: it is not part of `width`). A bar is at most `maxBar`
 * thick, centred in its row; a zero value is a 0-wide bar (the value still shows its 0).
 */
export function hbars(values: readonly number[], box: { left: number; top: number; width: number }, max: number, rowH = 30, maxBar = 16): HBar[] {
  if (values.length === 0 || max <= 0) return [];
  const thick = Math.min(maxBar, Math.max(2, rowH - 8));
  return values.map((value, i) => ({
    rowY: box.top + rowH * i, x: box.left, width: (Math.max(0, value) / max) * Math.max(0, box.width),
    y: box.top + rowH * i + (rowH - thick) / 2, height: thick, value,
  }));
}

/** x positions of `n` points over a plot box (one point sits in the middle) */
export function xs(n: number, left: number, width: number): number[] {
  if (n <= 0) return [];
  if (n === 1) return [left + width / 2];
  return Array.from({ length: n }, (_, i) => left + (width * i) / (n - 1));
}

/** y of a value in the plot box: 0 at the bottom, `max` at the top (values are never drawn below the bottom) */
export const yOf = (value: number, max: number, top: number, height: number): number => top + height - (Math.max(0, value) / max) * height;

const r2 = (n: number): string => String(Math.round(n * 100) / 100);

/**
 * A smooth monotone cubic path (Fritsch-Carlson) through the points: between two points the curve never leaves the range of
 * their y values, so it cannot dip below the lowest point (a 0 stays on the axis) or rise above a peak. One point: a move only.
 */
export function monotonePath(pts: ReadonlyArray<readonly [number, number]>): string {
  const n = pts.length;
  if (n === 0) return '';
  const first = pts[0]!;
  let d = `M${r2(first[0])} ${r2(first[1])}`;
  if (n === 1) return d;
  if (n === 2) return `${d}L${r2(pts[1]![0])} ${r2(pts[1]![1])}`;
  const dx: number[] = [];
  const sl: number[] = [];
  for (let i = 0; i < n - 1; i += 1) {
    const w = pts[i + 1]![0] - pts[i]![0];
    dx.push(w);
    sl.push(w === 0 ? 0 : (pts[i + 1]![1] - pts[i]![1]) / w);
  }
  const m: number[] = new Array<number>(n).fill(0);
  m[0] = sl[0]!;
  m[n - 1] = sl[n - 2]!;
  for (let i = 1; i < n - 1; i += 1) m[i] = sl[i - 1]! * sl[i]! > 0 ? (sl[i - 1]! + sl[i]!) / 2 : 0;
  for (let i = 0; i < n - 1; i += 1) {
    if (sl[i] === 0) {
      m[i] = 0;
      m[i + 1] = 0;
      continue;
    }
    const a = m[i]! / sl[i]!;
    const b = m[i + 1]! / sl[i]!;
    const s = a * a + b * b;
    if (s > 9) {
      const t = 3 / Math.sqrt(s);
      m[i] = t * a * sl[i]!;
      m[i + 1] = t * b * sl[i]!;
    }
  }
  for (let i = 0; i < n - 1; i += 1) {
    const p = pts[i]!;
    const q = pts[i + 1]!;
    const w = dx[i]! / 3;
    d += `C${r2(p[0] + w)} ${r2(p[1] + m[i]! * w)} ${r2(q[0] - w)} ${r2(q[1] - m[i + 1]! * w)} ${r2(q[0])} ${r2(q[1])}`;
  }
  return d;
}

/** The y of every control point of a path made by monotonePath (for the "never below the axis" check) */
export function controlYs(path: string): number[] {
  const out: number[] = [];
  for (const seg of path.split(/(?=[MCL])/)) {
    const nums = seg.slice(1).trim().split(/[\s,]+/).filter(Boolean).map(Number);
    for (let i = 1; i < nums.length; i += 2) out.push(nums[i]!);
  }
  return out;
}

/** The line closed down to the axis, for the soft fill under it */
export function areaPath(path: string, xs0: number, xs1: number, baseY: number): string {
  return path === '' ? '' : `${path}L${r2(xs1)} ${r2(baseY)}L${r2(xs0)} ${r2(baseY)}Z`;
}
