// The usage charts of AI settings > AI 用量 (v1.1, UC-1, A-OWNER-39 / 40): vertical bars by model and by feature (calls, or
// tokens stacked input + output) and a smooth line over time. Hand-made inline SVG (no chart library); the geometry is
// domain/usage-chart.ts and the rows are domain/usage-bars.ts (pure, tested). Colours are theme tokens (styles/ai-settings.css
// `.uc-*`), so light and dark follow. The numbers stay available in the (visually hidden) tables next to each chart. The
// drawing follows the width of its box (a resize redraws it), so the text keeps its size on a 360 px phone and on a desktop;
// crowded labels turn, never overlap and are never left out.
import {
  areaPath, axisMax, bars, fillPeriods, labelStep, monotonePath, rotateLabels, wrapLabel, xs, yOf, yTicks,
} from '../domain/usage-chart.ts';
import { formatBig, valueOf, type UsageMetric, type UsageRow } from '../domain/usage-bars.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { h } from '../utils/dom.ts';

const NS = 'http://www.w3.org/2000/svg';

function svgEl<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number>, text?: string): SVGElementTagNameMap[K] {
  const el = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  if (text != null) el.textContent = text;
  return el;
}

/** The estimated width of a label of `n` characters at the chart's 11 px type (CJK is wider) */
const labelWidth = (s: string): number => [...s].reduce((w, c) => w + (c.charCodeAt(0) > 0x2e80 ? 11 : 6.2), 0);

/** A label cut to `max` characters with an ellipsis (the full text is in the tip) */
export function cut(s: string, max: number): string {
  const a = [...s];
  return a.length <= max ? s : `${a.slice(0, Math.max(1, max - 1)).join('')}…`;
}

/** Draws the plot inside `host` through `draw(width)` now and again whenever the host's width changes */
function responsive(host: HTMLElement, draw: (width: number) => SVGSVGElement): void {
  const plot = document.createElement('div');
  host.append(plot);
  let last = 0;
  const width = (): number => Math.max(240, Math.floor(host.clientWidth || 320));
  const run = (): void => {
    const w = width();
    if (Math.abs(w - last) < 6 && plot.firstChild) return;
    last = w;
    plot.replaceChildren(draw(w));
  };
  plot.append(draw(width()));
  last = width();
  if (typeof ResizeObserver === 'function') new ResizeObserver(() => run()).observe(host);
}

/** The tip of a bar or point (hover, tap, focus): a small box in the chart's host, kept inside it. The first line is the name. */
interface Tip {
  show(x: number, y: number, lines: readonly string[]): void;
  hide(): void;
}

function makeTip(host: HTMLElement): Tip {
  const el = h('div', { class: 'uc-tip', role: 'status', 'aria-live': 'polite' });
  el.hidden = true;
  host.append(el);
  const hide = (): void => {
    el.hidden = true;
  };
  host.addEventListener('pointerleave', (e) => {
    if (e.pointerType === 'mouse') hide();
  });
  const outside = (e: Event): void => {
    if (!host.isConnected) document.removeEventListener('pointerdown', outside, true);
    else if (!(e.target instanceof Node && host.contains(e.target))) hide();
  };
  document.addEventListener('pointerdown', outside, true);
  return {
    hide,
    show(x, y, lines) {
      el.replaceChildren(...lines.map((l, i) => h('div', { class: i === 0 ? 'uc-tip-h' : 'uc-tip-l' }, l)));
      el.hidden = false;
      const w = el.offsetWidth;
      const hh = el.offsetHeight;
      el.style.left = `${Math.max(0, Math.min(host.clientWidth - w, x - w / 2))}px`;
      el.style.top = `${y - hh - 8 >= 0 ? y - hh - 8 : y + 16}px`;
    },
  };
}

export interface BarItem {
  label: string;
  value: number;
  /** Stacked parts (input, output tokens) that make `value` */
  parts?: readonly [number, number];
  /** The tip: the first line is the name */
  tip?: readonly string[];
}

/**
 * Vertical bars: x = the item, the value printed on top of its bar. Every bar has its label (long ones are cut with an
 * ellipsis, the full name is in the tip; crowded ones turn, they are never left out). A bar, its label and the space above
 * it show the tip on hover, tap and focus.
 */
export function barChart(o: { ariaLabel: string; items: readonly BarItem[]; format: (n: number) => string; maxLabel?: number }): HTMLElement {
  const host = document.createElement('div');
  host.className = 'uc-chart uc-bars';
  const tip = makeTip(host);
  responsive(host, (w) => {
    const H = 210;
    const n = o.items.length;
    const right = 8;
    const top = 22;
    const max = Math.max(1, ...o.items.map((i) => i.value));
    let left = 8;
    let slot = (w - left - right) / Math.max(1, n);
    // one layout for every language: labels wrap into up to three lines inside their slot (the full text is in the tip); only when a
    // Latin word cannot fit its slot (a phone) do the labels slant, cut shorter when that would squeeze the plot
    const wrapped = o.items.map((i) => wrapLabel(i.label, slot));
    const wrap = wrapped.every((l) => l != null);
    const lineH = 13;
    let maxLen = o.maxLabel ?? 16;
    let labels = o.items.map((i) => (wrap ? i.label : cut(i.label, maxLen)));
    let widest = Math.max(0, ...labels.map(labelWidth));
    let turn = !wrap && rotateLabels(slot, widest);
    let steep = turn && slot < 13;
    const rows = wrap ? Math.max(1, ...wrapped.map((l) => l!.length)) : 1;
    let bottom = wrap ? 12 + rows * lineH : turn ? 14 + widest * (steep ? 0.9 : 0.72) : 26;
    while (turn && bottom > 110 && maxLen > 6) {
      maxLen -= 2;
      labels = o.items.map((i) => cut(i.label, maxLen));
      widest = Math.max(0, ...labels.map(labelWidth));
      turn = rotateLabels(slot, widest);
      steep = turn && slot < 13;
      bottom = turn ? 14 + widest * (steep ? 0.9 : 0.72) : 26;
    }
    // a slanted label runs down to the left of its bar: the first ones need room on the left, or they would leave the card
    if (turn) {
      const across = steep ? 0.47 : 0.79;
      const need = Math.max(...labels.map((l, i) => labelWidth(l) * across + 12 - (slot * (i + 0.5) + 3)));
      left = Math.max(8, Math.ceil(need) + 4);
      slot = (w - left - right) / Math.max(1, n);
    }
    const box = { left, top, width: w - left - right, height: H - top - bottom };
    const svg = svgEl('svg', { viewBox: `0 0 ${w} ${H}`, width: w, height: H, role: 'group', 'aria-label': o.ariaLabel, class: 'uc-svg' });
    svg.append(svgEl('line', { x1: left, y1: top + box.height, x2: w - right, y2: top + box.height, class: 'uc-axis' }));
    // the numbers on top are thinned when they would touch; the labels never are
    const valStep = labelStep(slot, Math.max(0, ...o.items.map((i) => o.format(i.value).length)) * 6.6);
    bars(o.items.map((i) => i.value), box, max).forEach((b, i) => {
      const it = o.items[i]!;
      const lines = it.tip ?? [it.label, o.format(it.value)];
      const g = svgEl('g', { class: 'uc-item', tabindex: 0, role: 'img', 'aria-label': lines.join(', ') });
      const cx = b.x + b.width / 2;
      const showTip = (): void => tip.show(cx, b.y, lines);
      g.addEventListener('pointerenter', showTip);
      g.addEventListener('click', showTip);
      g.addEventListener('focus', showTip);
      g.addEventListener('blur', () => tip.hide());
      // the whole slot (bar, label and the space above) is the target
      g.append(svgEl('rect', { x: left + slot * i, y: 0, width: slot, height: H, class: 'uc-hit' }));
      if (it.parts && it.value > 0) {
        const outH = (it.parts[1] / it.value) * b.height;
        const inH = b.height - outH;
        g.append(svgEl('rect', { x: b.x, y: b.y, width: b.width, height: Math.max(outH, 0), rx: 2, class: 'uc-bar' }));
        g.append(svgEl('rect', { x: b.x, y: b.y + outH, width: b.width, height: Math.max(inH, 0), class: 'uc-bar uc-bar-in' }));
      } else {
        g.append(svgEl('rect', { x: b.x, y: b.y, width: b.width, height: Math.max(b.height, 0.5), rx: 2, class: 'uc-bar' }));
      }
      if (i % valStep === 0) g.append(svgEl('text', { x: cx, y: b.y - 5, 'text-anchor': 'middle', class: 'uc-val' }, o.format(it.value)));
      const ly = top + box.height + 14;
      if (wrap) {
        const lab = svgEl('text', { x: cx, y: ly, 'text-anchor': 'middle', class: 'uc-label' });
        wrapped[i]!.forEach((line, k) => lab.append(svgEl('tspan', { x: cx, dy: k === 0 ? 0 : lineH }, line)));
        g.append(lab);
      } else {
        g.append(svgEl('text', turn
          ? { x: cx + 3, y: ly - 2, 'text-anchor': 'end', transform: `rotate(${steep ? -62 : -38} ${cx + 3} ${ly - 2})`, class: 'uc-label' }
          : { x: cx, y: ly, 'text-anchor': 'middle', class: 'uc-label' }, labels[i]!));
      }
      svg.append(g);
    });
    return svg;
  });
  return host;
}

/** The remembered metric of a chart (per browser; a blocked store just means the default) */
function readMetric(pref: string): UsageMetric {
  try {
    return localStorage.getItem(`xexam.uc.${pref}`) === 'tokens' ? 'tokens' : 'calls';
  } catch {
    return 'calls';
  }
}

function writeMetric(pref: string, m: UsageMetric): void {
  try {
    localStorage.setItem(`xexam.uc.${pref}`, m);
  } catch {
    // not remembered
  }
}

/** The tip lines of one row (exact numbers) */
function rowTip(r: UsageRow, metric: UsageMetric, exact: (n: number) => string): string[] {
  const lines = [r.label, t('ai.set.uc.tip.calls', { n: exact(r.calls) }), t('ai.set.uc.tip.in', { n: exact(r.tokensIn) }), t('ai.set.uc.tip.out', { n: exact(r.tokensOut) })];
  if (metric === 'tokens') lines.push(t('ai.set.uc.tip.total', { n: exact(r.tokensIn + r.tokensOut) }));
  lines.push(t('ai.set.uc.tip.err', { n: exact(r.errors) }));
  // A-OWNER-74: the cache figures ride in the tip, not in a new column
  lines.push(t('ai.set.cacheLine', { r: r.tokensCached ? exact(r.tokensCached) : '—', w: r.tokensCacheWrite ? exact(r.tokensCacheWrite) : '—' }));
  return lines;
}

/**
 * A chart card: its title and the 调用次数 | Token 数 switch at the top right (A-OWNER-40 / 40b / 63), the legend 输入 / 输出 in
 * token mode, and the chart `draw` makes for the chosen metric. `pref` names the remembered choice, `titleKey` the title.
 */
function metricCard(o: { line?: boolean; pref: string; titleKey: (m: UsageMetric) => MsgKey; draw: (m: UsageMetric) => HTMLElement }): HTMLElement {
  let metric = readMetric(o.pref);
  const metrics = ['calls', 'tokens'] as const;
  const title = h('h3', { class: 'ais-h3 uc-title' }, t(o.titleKey(metric)));
  const slot = h('div', { class: 'uc-slot' });
  const legend = h('div', { class: 'uc-legend', 'aria-hidden': 'true' },
    h('span', { class: 'uc-key' }, h('i', { class: o.line ? 'uc-sw uc-sw-lin' : 'uc-sw uc-sw-in' }), t('ai.set.uc.in')),
    h('span', { class: 'uc-key' }, h('i', { class: o.line ? 'uc-sw uc-sw-lout' : 'uc-sw uc-sw-out' }), t('ai.set.uc.out')));
  const draw = (): void => {
    title.textContent = t(o.titleKey(metric));
    legend.hidden = metric !== 'tokens';
    // the chart is drawn a moment after it is attached: keep the height meanwhile, so the page does not collapse and jump under the finger
    if (slot.offsetHeight > 0) {
      slot.style.minHeight = `${slot.offsetHeight}px`;
      window.setTimeout(() => { slot.style.minHeight = ''; }, 300);
    }
    slot.replaceChildren(o.draw(metric));
  };
  const btns = metrics.map((m) => h('button', {
    type: 'button', class: 'uc-sw-btn', 'aria-pressed': String(m === metric),
    onclick: () => {
      if (metric === m) return;
      metric = m;
      writeMetric(o.pref, m);
      btns.forEach((b, i) => b.setAttribute('aria-pressed', String(metrics[i] === metric)));
      draw();
    },
  }, t(m === 'calls' ? 'ai.set.uc.calls' : 'ai.set.uc.tokens')));
  const card = h('div', { class: 'uc-card' },
    h('div', { class: 'uc-head' }, title, h('div', { class: 'uc-switch', role: 'group', 'aria-label': t('ai.set.uc.switchLabel') }, ...btns)),
    legend, slot);
  draw();
  return card;
}

/**
 * A bar chart card (A-OWNER-40 / 40b): calls as plain bars, tokens as bars stacked input + output (two shades, a small legend).
 * `kind` names the title keys.
 */
export function usageBarCard(o: { pref: string; kind: 'model' | 'purpose'; rows: readonly UsageRow[]; maxLabel: number; exact: (n: number) => string }): HTMLElement {
  const titleKey = (m: UsageMetric): MsgKey => `ai.set.uc.${o.kind}.${m}` as MsgKey;
  return metricCard({
    pref: o.pref, titleKey,
    draw: (metric) => barChart({
      ariaLabel: t(titleKey(metric)), maxLabel: o.maxLabel, format: (n) => formatBig(n),
      items: o.rows.map((r) => ({
        label: r.label, value: valueOf(r, metric), tip: rowTip(r, metric, o.exact),
        ...(metric === 'tokens' ? { parts: [r.tokensIn, r.tokensOut] as const } : {}),
      })),
    }),
  });
}

type PeriodRow = { period: string; calls: number; tokensIn?: number; tokensOut?: number; errors?: number };

/** The 按时间 card (A-OWNER-63): the line of calls, or the tokens as two stacked areas input + output, with the same switch */
export function usageLineCard(o: { unit: 'day' | 'month'; from: string; to: string; rows: ReadonlyArray<PeriodRow>; format: (n: number) => string }): HTMLElement {
  const titleKey = (m: UsageMetric): MsgKey => (m === 'tokens' ? 'ai.set.uc.period.tokens' : 'ai.set.uc.period');
  return metricCard({
    line: true, pref: 'period', titleKey,
    draw: (metric) => lineChart({ ariaLabel: t(titleKey(metric)), unit: o.unit, from: o.from, to: o.to, rows: o.rows, format: o.format, metric }),
  });
}

/**
 * The calls (or the tokens, A-OWNER-63) over time: a smooth monotone line with a soft fill. `rows` are the server's `byPeriod` rows; the gaps between
 * `from` and `to` are filled with 0 (full `YYYY-MM-DD` labels for unit day, `YYYY-MM` for unit month). The line is 3 px with a
 * hit area of 16 px around it: hover, tap or the arrow keys show the date, calls and tokens of the nearest period.
 */
export function lineChart(o: {
  ariaLabel: string; unit: 'day' | 'month'; from: string; to: string;
  rows: ReadonlyArray<PeriodRow>;
  format: (n: number) => string;
  /** 'tokens': the total input + output as the line, the input below it as a lighter area (A-OWNER-63); default 'calls' */
  metric?: UsageMetric;
}): HTMLElement {
  const tokens = o.metric === 'tokens';
  const host = document.createElement('div');
  host.className = 'uc-chart uc-line';
  const tip = makeTip(host);
  const byPeriod = new Map(o.rows.map((r) => [r.period, r]));
  const pts = fillPeriods(o.unit, o.from, o.to, o.rows);
  const tokIn = (period: string): number => byPeriod.get(period)?.tokensIn ?? 0;
  const tokAll = (period: string): number => tokIn(period) + (byPeriod.get(period)?.tokensOut ?? 0);
  // the plotted value of a period: its calls, or its input + output tokens
  const val = (p: { period: string; calls: number }): number => (tokens ? tokAll(p.period) : p.calls);
  const tipOf = (period: string): string[] => {
    const r = byPeriod.get(period);
    const lines = [period, t('ai.set.uc.tip.calls', { n: o.format(r?.calls ?? 0) }), t('ai.set.uc.tip.in', { n: o.format(r?.tokensIn ?? 0) }),
      t('ai.set.uc.tip.out', { n: o.format(r?.tokensOut ?? 0) })];
    if (tokens) lines.push(t('ai.set.uc.tip.total', { n: o.format(tokAll(period)) }));
    lines.push(t('ai.set.uc.tip.err', { n: o.format(r?.errors ?? 0) }));
    return lines;
  };
  let kb = pts.length - 1;
  responsive(host, (w) => {
    const H = 210;
    const n = pts.length;
    const max = Math.max(1, ...pts.map(val));
    const ticks = yTicks(max);
    const tickText = (v: number): string => formatBig(v);
    const top = 14;
    const axis = axisMax(max);
    const widestY = Math.max(...ticks.map((tk) => tickText(tk).length)) * 6.4 + 10;
    const left = Math.round(widestY) + 4;
    const right = 12;
    const lw = Math.max(...pts.map((p) => labelWidth(p.period)), 10);
    const slot = (w - left - right) / Math.max(1, n - 1);
    // a label slanted by 38 degrees takes cos(38) of its length plus sin(38) of its height across: thinned by that box
    const step = labelStep(slot, lw * 0.79 + 7);
    const bottom = 14 + lw * 0.72 * 0.74 + 8;
    const plotW = w - left - right;
    const plotH = H - top - bottom;
    const svg = svgEl('svg', { viewBox: `0 0 ${w} ${H}`, width: w, height: H, role: 'img', 'aria-label': o.ariaLabel, class: 'uc-svg', tabindex: 0 });
    const px = xs(n, left, plotW);
    const points = pts.map((p, i): [number, number] => [px[i]!, yOf(val(p), axis, top, plotH)]);
    const line = monotonePath(points);
    // the fills go first (token mode: opaque, the input area over the total one), the grid over them, the lines on top
    if (n > 1) svg.append(svgEl('path', { d: areaPath(line, px[0]!, px[n - 1]!, top + plotH), class: tokens ? 'uc-area uc-area-out' : 'uc-area' }));
    let inLine = '';
    if (tokens && n > 1) {
      inLine = monotonePath(pts.map((p, i): [number, number] => [px[i]!, yOf(tokIn(p.period), axis, top, plotH)]));
      svg.append(svgEl('path', { d: areaPath(inLine, px[0]!, px[n - 1]!, top + plotH), class: 'uc-area uc-area-in' }));
    }
    for (const tick of ticks) {
      const y = yOf(tick, axis, top, plotH);
      svg.append(svgEl('line', { x1: left, y1: y, x2: w - right, y2: y, class: tick === 0 ? 'uc-axis' : 'uc-grid' }));
      svg.append(svgEl('text', { x: left - 6, y: y + 3.5, 'text-anchor': 'end', class: 'uc-label' }, tickText(tick)));
    }
    if (inLine) svg.append(svgEl('path', { d: inLine, class: 'uc-stroke uc-stroke-in', fill: 'none' }));
    svg.append(svgEl('path', { d: line, class: 'uc-stroke', fill: 'none' }));
    const hot = svgEl('circle', { r: 6, class: 'uc-hot' });
    hot.style.display = 'none';
    pts.forEach((p, i) => {
      const [x, y] = points[i]!;
      if (n <= 45 || val(p) > 0) svg.append(svgEl('circle', { cx: x, cy: y, r: 3.4, class: 'uc-dot' }));
      // counted from the newest period back, so the last day is always named and the labels never touch
      // a slanted label runs down to the left: one that would reach under the y labels (the oldest periods) is left out
      if ((n - 1 - i) % step === 0 && x - labelWidth(p.period) * 0.79 >= left - 4) {
        const ly = top + plotH + 14;
        svg.append(svgEl('text', { x, y: ly - 2, 'text-anchor': 'end', transform: `rotate(-38 ${x} ${ly - 2})`, class: 'uc-label' }, p.period));
      }
    });
    svg.append(hot);
    const showAt = (i: number): void => {
      const [x, y] = points[i]!;
      hot.setAttribute('cx', String(x));
      hot.setAttribute('cy', String(y));
      hot.style.display = '';
      tip.show(x, y, tipOf(pts[i]!.period));
    };
    const hideAll = (): void => {
      hot.style.display = 'none';
      tip.hide();
    };
    // the hit area: the pointer within HIT px (up or down) of the line, or of a point; the tip is the nearest period by x
    const HIT = 16;
    const lineY = (x: number): number => {
      let j = 0;
      while (j < n - 2 && points[j + 1]![0] < x) j++;
      const a = points[j]!;
      const b = points[Math.min(n - 1, j + 1)]!;
      return b[0] === a[0] ? a[1] : a[1] + ((b[1] - a[1]) * (x - a[0])) / (b[0] - a[0]);
    };
    const move = (e: PointerEvent): void => {
      const r = svg.getBoundingClientRect();
      const k = r.width > 0 ? w / r.width : 1;
      const x = (e.clientX - r.left) * k;
      const y = (e.clientY - r.top) * k;
      if (n === 0 || x < left - HIT || x > w - right + HIT) return hideAll();
      let best = 0;
      for (let i = 1; i < n; i++) if (Math.abs(px[i]! - x) < Math.abs(px[best]! - x)) best = i;
      const onPoint = Math.hypot(px[best]! - x, points[best]![1] - y) <= HIT;
      const ly = lineY(Math.min(w - right, Math.max(left, x)));
      // token mode: the input band under the total line is part of the hit area (down to the axis)
      const onLine = Math.abs(y - ly) <= HIT || (tokens && y >= ly && y <= top + plotH + HIT);
      if (onPoint || onLine) {
        kb = best;
        showAt(best);
      } else hideAll();
    };
    svg.addEventListener('pointermove', move);
    svg.addEventListener('pointerdown', move);
    svg.addEventListener('pointerleave', (e) => {
      if (e.pointerType === 'mouse') hideAll();
    });
    svg.addEventListener('blur', hideAll);
    svg.addEventListener('keydown', (e) => {
      const d = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : e.key === 'Home' ? -n : e.key === 'End' ? n : 0;
      if (!d || n === 0) return;
      e.preventDefault();
      kb = Math.max(0, Math.min(n - 1, kb + d));
      showAt(kb);
    });
    return svg;
  });
  return host;
}
