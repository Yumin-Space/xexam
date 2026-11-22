// v2.6 Home leaderboard as a hand-drawn line chart (owner): x and y axes and one straight rising line, like a linear function
// drawn on the board; place 1 stands top right, the next places lower left along the same line, the viewer's own place (when
// outside the top three) lowest. Each point carries its place, name and correct answers; a tap / click / hover / visible
// focus shows the full line under the board. Same board, strokes and font as the most-missed board (components/chalk-board.ts).
import type { RankEntry, RankMe } from '../api/dashboard.ts';
import { BOARD_H_PHONE, BOARD_H_WIDE, BOARD_WIDE_FROM, labelWidth } from '../domain/dash.ts';
import { circlePoly, hachure, rng, sketchCircle, sketchLine } from '../domain/sketch.ts';
import { t } from '../i18n/index.ts';
import { h } from '../utils/dom.ts';
import { onClearUserData } from '../state/store.ts';
import { fitBoard, sizeBoard, strokes, svg } from './chalk-board.ts';


/** One point of the chart: a ranked user (the viewer's own row too) */
export interface RankPoint {
  key: string;
  rank: number;
  name: string;
  answered: number;
  correct: number;
  self: boolean;
}

/** The points in board order, best first: the top three, then the viewer when ranked outside them */
export function rankPoints(top: readonly RankEntry[], me: RankMe): RankPoint[] {
  const pts: RankPoint[] = top.map((e) => ({ key: `r${e.rank}`, rank: e.rank, name: e.name, answered: e.answered, correct: e.correct, self: e.self }));
  if (me.rank !== null && !top.some((e) => e.self)) {
    pts.push({ key: 'me', rank: me.rank, name: t('dash.rank.you'), answered: me.answered, correct: me.correct, self: true });
  }
  return pts;
}

const PAD = 26;

/** Cuts a name so it fits `room` px at `px` (an ellipsis marks the cut); the full name is in the detail line */
function fitName(text: string, px: number, room: number): string {
  if (labelWidth(text, px) <= room) return text;
  const chars = [...text];
  while (chars.length > 1 && labelWidth(chars.join('') + '…', px) > room) chars.pop();
  return chars.join('') + '…';
}

let picked: { hover: string | null; pinned: string | null } = { hover: null, pinned: null };
onClearUserData(() => { picked = { hover: null, pinned: null }; });

/** The name with the "You" mark on the viewer's own row */
const nameOf = (p: RankPoint): string => (p.self && p.key !== 'me' ? t('dash.rank.nameYou', { name: p.name }) : p.name);

/** Loading: the empty axes at the real board's height and with an empty detail line, so the page does not jump */
export function rankBoardSkeleton(): HTMLElement {
  const art = svg('svg', { class: 'dash-chalk', 'aria-hidden': 'true', focusable: 'false' });
  const board = h('div', { class: 'dash-board dash-rank-board dash-board-skel' }, art);
  fitBoard(board, (width) => {
    const w = Math.max(220, Math.round(width));
    const hh = w >= BOARD_WIDE_FROM ? BOARD_H_WIDE : BOARD_H_PHONE;
    const box = { w, h: hh };
    sizeBoard(board, art, box);
    const o = { x: PAD, y: hh - PAD };
    art.replaceChildren(
      ...strokes(sketchLine(o, { x: w - 10, y: o.y }, rng(11), box), 'dash-chalk-axis'),
      ...strokes(sketchLine(o, { x: o.x, y: 12 }, rng(12), box), 'dash-chalk-axis'));
  });
  return h('div', { class: 'dash-board-wrap', 'aria-hidden': 'true' }, board, h('div', { class: 'dash-pick' }));
}

/** The leaderboard board and its detail line */
export function rankBoard(points: readonly RankPoint[]): HTMLElement {
  // Only a pick whose point is gone is dropped (a repaint of Home must keep a hover)
  const keys = points.map((p) => p.key);
  if (!keys.includes(picked.pinned ?? '')) picked.pinned = null;
  if (!keys.includes(picked.hover ?? '')) picked.hover = null;
  const detail = h('div', { class: 'dash-pick dash-pick-stack dash-rank-pick', role: 'region', 'aria-label': t('dash.rank.title') });
  const buttons = new Map<string, HTMLButtonElement>();
  const art = svg('svg', { class: 'dash-chalk', 'aria-hidden': 'true', focusable: 'false' });
  const ring = svg('g', { class: 'dash-chalk-ring' });
  let spots: { key: string; x: number; y: number; r: number }[] = [];
  let box = { w: 300, h: BOARD_H_PHONE };
  const shown = (): string | null => picked.hover ?? picked.pinned;
  const describe = (p: RankPoint): string =>
    t('dash.rank.detail', { rank: p.rank, name: nameOf(p), answered: p.answered, correct: p.correct });
  const paint = (): void => {
    const id = shown();
    const p = points.find((x) => x.key === id);
    if (detail.childElementCount === 0) {
      // every line drawn once, layered in one cell (see chalk-board.ts pickLayers): hovering never changes the height
      detail.append(h('div', { class: 'dash-pick-layer', 'data-k': '' }, h('p', { class: 'dim dash-pick-hint' }, t('dash.rank.hint'))),
        ...points.map((q) => h('div', { class: 'dash-pick-layer', 'data-k': q.key, 'aria-hidden': 'true' }, h('p', { class: 'dash-rank-line' }, describe(q)))),
        h('p', { class: 'sr-only rank-live', 'aria-live': 'polite' }));
    }
    for (const l of Array.from(detail.querySelectorAll<HTMLElement>('.dash-pick-layer'))) {
      const on = (l.dataset.k || null) === (p ? p.key : null);
      l.classList.toggle('on', on);
      l.toggleAttribute('aria-hidden', !on);
    }
    const live = detail.querySelector('.rank-live');
    if (live) live.textContent = p ? describe(p) : '';
    for (const [k, b] of buttons) {
      b.setAttribute('aria-pressed', String(picked.pinned === k));
      b.classList.toggle('on', id === k);
    }
    const s = spots.find((x) => x.key === id);
    ring.replaceChildren(...(s ? strokes(sketchCircle({ x: s.x, y: s.y }, s.r + 8, rng(500 + s.r), box), 'dash-chalk-hi') : []));
  };
  const items = points.map((p) => {
    const b = h('button', {
      id: `dash-pt-${p.key}`, class: 'dash-pt', type: 'button', 'aria-pressed': 'false', 'aria-label': describe(p),
      onclick: () => { picked.pinned = picked.pinned === p.key ? null : p.key; paint(); },
      onpointerenter: (e: PointerEvent) => { if (e.pointerType === 'mouse') { picked.hover = p.key; paint(); } },
      onpointerleave: (e: PointerEvent) => { if (e.pointerType === 'mouse' && picked.hover === p.key) { picked.hover = null; paint(); } },
      onfocus: (e: FocusEvent) => { if ((e.target as HTMLElement).matches(':focus-visible')) { picked.hover = p.key; paint(); } },
      onblur: () => { if (picked.hover === p.key) { picked.hover = null; paint(); } },
    }) as HTMLButtonElement;
    buttons.set(p.key, b);
    return h('li', null, b);
  });
  const board = h('div', { class: 'dash-board dash-rank-board' }, art, h('ol', { class: 'dash-pts', 'aria-label': t('dash.rank.title') }, ...items));
  const draw = (width: number): void => {
    const w = Math.max(220, Math.round(width));
    const hh = w >= BOARD_WIDE_FROM ? BOARD_H_WIDE : BOARD_H_PHONE;
    box = { w, h: hh };
    sizeBoard(board, art, box);
    // The axes: the origin bottom left, arrows to the right (x) and up (y)
    const o = { x: PAD, y: hh - PAD };
    const xEnd = { x: w - 10, y: o.y };
    const yEnd = { x: o.x, y: 12 };
    const lines = svg('g', {},
      ...strokes(sketchLine(o, xEnd, rng(11), box), 'dash-chalk-axis'),
      ...strokes(sketchLine(o, yEnd, rng(12), box), 'dash-chalk-axis'),
      ...strokes(sketchLine(xEnd, { x: xEnd.x - 8, y: xEnd.y - 5 }, rng(13), box), 'dash-chalk-axis'),
      ...strokes(sketchLine(xEnd, { x: xEnd.x - 8, y: xEnd.y + 5 }, rng(14), box), 'dash-chalk-axis'),
      ...strokes(sketchLine(yEnd, { x: yEnd.x - 5, y: yEnd.y + 8 }, rng(15), box), 'dash-chalk-axis'),
      ...strokes(sketchLine(yEnd, { x: yEnd.x + 5, y: yEnd.y + 8 }, rng(16), box), 'dash-chalk-axis'));
    // The function itself: one straight rising line across the board
    const a = { x: o.x + 18, y: o.y - 16 };
    const z = { x: w - 30, y: 30 };
    lines.append(...strokes(sketchLine(a, z, rng(17), box), 'dash-chalk-curve'));
    const texts = svg('g', { class: 'dash-chalk-text' },
      svg('text', { x: xEnd.x - 2, y: o.y + 16, 'text-anchor': 'end', class: 'dash-curve-t' }, document.createTextNode('x')),
      svg('text', { x: o.x - 12, y: yEnd.y + 6, 'text-anchor': 'middle', class: 'dash-curve-t' }, document.createTextNode('y')));
    // Best first: place 1 near the top right end, the rest lower left along the line
    const n = points.length;
    spots = [];
    // The bottom of the previous point's tap target: the next target never reaches up over it (or over its words)
    let prevY1 = 0;
    points.forEach((p, i) => {
      const tt = n === 1 ? 0.85 : 0.9 - (i * 0.75) / (n - 1);
      const x = a.x + (z.x - a.x) * tt;
      const y = a.y + (z.y - a.y) * tt;
      const r = i === 0 ? 7 : 5;
      spots.push({ key: p.key, x, y, r });
      lines.append(
        svg('path', { d: hachure(circlePoly({ x, y }, r - 0.6), 1.8, -41, rng(600 + i), box), class: 'dash-chalk-fill' }),
        ...strokes(sketchCircle({ x, y }, r, rng(700 + i), box), 'dash-chalk-dot'));
      // The upper points carry their words above-left of the dot (the line climbs away to the right), the lower ones
      // below-right (the line runs away to the left there): neither crosses the line
      const upper = tt >= 0.5;
      // Measured at the size the name is drawn in (14 px, 16 px for place 1), a little wider for the semibold weight
      const px = i === 0 ? 16 : 14;
      const name = `${p.rank}  ${nameOf(p)}`;
      const sub = t('dash.rank.correctN', { n: p.correct });
      const room = upper ? x - 12 - (o.x + 6) : w - 6 - (x + 12);
      const nameShown = fitName(name, px, Math.max(40, room) / 1.05);
      const tx = upper ? x - 12 : x + 12;
      const anchor = upper ? 'end' : 'start';
      const ty = upper ? y - 18 : y + 16;
      texts.append(
        svg('text', { x: tx, y: ty, 'text-anchor': anchor, class: i === 0 ? 'dash-curve-n dash-curve-n1' : 'dash-curve-n' }, document.createTextNode(nameShown)),
        svg('text', { x: tx, y: ty + 15, 'text-anchor': anchor, class: 'dash-curve-t' }, document.createTextNode(sub)));
      const b = buttons.get(p.key);
      if (b) {
        const lw = Math.max(labelWidth(nameShown, px), labelWidth(sub, 12));
        const x0 = Math.max(0, upper ? Math.min(x - 22, tx - lw) : x - 22);
        const x1 = Math.min(w, upper ? x + 22 : Math.max(x + 22, tx + lw));
        const y1 = Math.min(hh, upper ? y + 22 : ty + 20);
        const y0 = Math.max(0, Math.min(y1 - 44, Math.max(upper ? ty - 16 : y - 22, prevY1)));
        prevY1 = y1;
        b.style.left = `${x0}px`;
        b.style.top = `${y0}px`;
        b.style.width = `${x1 - x0}px`;
        b.style.height = `${y1 - y0}px`;
      }
    });
    lines.append(ring);
    art.replaceChildren(lines, texts);
    paint();
  };
  fitBoard(board, draw);
  const wrap = h('div', {
    class: 'dash-board-wrap',
    onkeydown: (e: KeyboardEvent) => { if (e.key === 'Escape' && picked.pinned) { picked.pinned = null; paint(); } },
  }, board, detail);
  paint();
  return wrap;
}
