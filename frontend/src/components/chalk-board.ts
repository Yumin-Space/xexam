// v2.6 Home "most missed questions": the places stand on a hand-drawn parabola on a small black board (place 1 at the
// vertex, 2 / 3 lower on either side, 4 / 5 lowest at the edges; no award symbols). The drawing is an aria-hidden SVG redrawn for
// the board's real width (so texts and dots keep their size); the interaction is real <button>s laid over the points. The
// strokes come from domain/sketch.ts (seeded, so a repaint draws the very same lines); the geometry from domain/dash.ts.
// Mouse hover or visible keyboard focus shows a question, a tap / click pins it (again unpins), Escape clears a pin; the
// detail region under the board is aria-live polite.
import { htmlLang } from '../domain/language.ts';
import { boardLayout, detailId, podiumOrder, togglePin, type BoardLayout, type WrongRow } from '../domain/dash.ts';
import { circlePoly, hachure, rng, sketchCircle, sketchCurve } from '../domain/sketch.ts';
import { t } from '../i18n/index.ts';
import { onClearUserData } from '../state/store.ts';
import { h } from '../utils/dom.ts';

/** The picked point: `hover` = a mouse over it or the keyboard on it, `pinned` = a tap or click; both survive a repaint */
const pick: { hover: string | null; pinned: string | null } = { hover: null, pinned: null };

onClearUserData(() => { pick.hover = null; pick.pinned = null; });

/** Drops a pick of an earlier visit whose question is no longer on the board */
export function keepPick(ids: readonly string[]): void {
  if (!ids.includes(pick.pinned ?? '')) pick.pinned = null;
  if (!ids.includes(pick.hover ?? '')) pick.hover = null;
}

/** The last width a board was drawn at: the next render starts from it, so a repaint does not jump */
let boardWidth = 270;

const SVGNS = 'http://www.w3.org/2000/svg';

export function svg<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}, ...kids: Node[]): SVGElementTagNameMap[K] {
  const el = document.createElementNS(SVGNS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  el.append(...kids);
  return el;
}

/** The two strokes of a sketched line: the first firm, the second a lighter retrace */
export const strokes = (ds: readonly string[], cls: string): SVGPathElement[] =>
  ds.map((d, i) => svg('path', { d, class: `${cls} dash-s${i + 1}` }));

/**
 * Keeps a board drawn at its real width: draws once at the last known width, then again whenever the width changes (a
 * ResizeObserver; it stops once the board has left the page).
 */
export function fitBoard(board: HTMLElement, draw: (w: number) => void): void {
  let drawn = boardWidth;
  draw(drawn);
  if (typeof ResizeObserver === 'undefined') return;
  const ro = new ResizeObserver(() => {
    if (!board.isConnected) { ro.disconnect(); return; }
    const w = Math.round(board.clientWidth);
    if (w > 0 && w !== drawn) { drawn = w; boardWidth = w; draw(w); }
  });
  ro.observe(board);
}

/** Sizes the SVG and the board to a layout */
export function sizeBoard(board: HTMLElement, art: SVGSVGElement, lay: { w: number; h: number }): void {
  art.setAttribute('viewBox', `0 0 ${lay.w} ${lay.h}`);
  art.setAttribute('width', String(lay.w));
  art.setAttribute('height', String(lay.h));
  board.style.height = `${lay.h}px`;
}

/** The chalk curve on its own (the board of the loading state and the base of the real one) */
const curvePaths = (lay: BoardLayout): SVGPathElement[] =>
  strokes(sketchCurve(lay.curve, rng(7), lay, { jitter: 1.4 }), 'dash-chalk-curve');

/** The detail region's content: the shown point's question, or the hint before any pick */
function pickBody(row: WrongRow, stats: (row: WrongRow) => string): HTMLElement[] {
  return [
    h('p', { class: 'dash-pick-stem', lang: htmlLang(row.lang) }, row.stem),
    ...(row.answer.length ? [h('div', { class: 'dash-pick-ans' },
      h('span', { class: 'dash-pick-ans-h' }, t('result.q.correctTag')),
      ...row.answer.map((line) => h('p', { class: 'dash-pick-ans-l', lang: htmlLang(row.lang) }, line)))] : []),
    h('p', { class: 'dash-pick-meta' }, stats(row)),
  ];
}

/**
 * The detail region's layers: the hint and one layer per question, all in the same grid cell. Only the shown layer is
 * visible; with a mouse the hidden ones still take their room (the region is as tall as the tallest question, so moving
 * from point to point never changes the page height — owner: the page jumped up and down), on touch screens they are left
 * out of the layout (no tall empty box under the hint). A screen-reader line repeats the shown question.
 */
function pickLayers(rows: readonly WrongRow[], stats: (row: WrongRow) => string): { el: HTMLElement[]; show: (id: string | null) => void } {
  const hint = h('div', { class: 'dash-pick-layer on' }, h('p', { class: 'dim dash-pick-hint' }, t('dash.wrong.hint')));
  const layers = new Map(rows.map((r) => [r.id, h('div', { class: 'dash-pick-layer', 'aria-hidden': 'true' }, ...pickBody(r, stats))]));
  const live = h('p', { class: 'sr-only', 'aria-live': 'polite' });
  const show = (id: string | null): void => {
    const on = id ? layers.get(id) ?? null : null;
    hint.classList.toggle('on', !on);
    hint.toggleAttribute('aria-hidden', !!on);
    for (const [k, l] of layers) {
      l.classList.toggle('on', l === on);
      l.toggleAttribute('aria-hidden', l !== on);
      void k;
    }
    const row = id ? rows.find((r) => r.id === id) : undefined;
    live.textContent = row ? `${row.stem} ${row.answer.length ? t('result.q.correctTag') + ' ' + row.answer.join(' / ') : ''}` : '';
  };
  return { el: [hint, ...layers.values(), live], show };
}

const missesText = (n: number): string => (n === 1 ? t('dash.wrong.miss') : t('dash.wrong.misses', { n }));

/** The board and its detail region; `stats` is the muted line under the stem (bank, sheet, answered, missed) */
export function chalkBoard(rows: readonly WrongRow[], stats: (row: WrongRow) => string): HTMLElement {
  const detail = h('div', { class: 'dash-pick dash-pick-stack', role: 'region', 'aria-label': t('dash.wrong.region') });
  const layers = pickLayers(rows, stats);
  detail.append(...layers.el);
  const labels: Record<number, string> = {};
  for (const r of rows) labels[r.rank] = missesText(r.wrong);
  const buttons = new Map<string, HTMLButtonElement>();
  const rankOf = new Map(rows.map((r) => [r.id, r.rank]));
  let lay = boardLayout(boardWidth, labels);
  // The ring around the shown point lives in its own group: hover, focus and taps redraw only it (and the buttons' state)
  const ring = svg('g', { class: 'dash-chalk-ring' });
  const drawRing = (): void => {
    const rank = rankOf.get(detailId(pick.hover, pick.pinned) ?? '');
    const p = lay.places.find((x) => x.rank === rank);
    ring.replaceChildren(...(p ? strokes(sketchCircle({ x: p.x, y: p.y }, p.r + 8, rng(200 + p.rank), lay), 'dash-chalk-hi') : []));
  };
  const paint = (): void => {
    const shownId = detailId(pick.hover, pick.pinned);
    layers.show(shownId);
    for (const [id, b] of buttons) {
      b.setAttribute('aria-pressed', String(pick.pinned === id));
      b.classList.toggle('on', shownId === id);
    }
    drawRing();
  };
  const place = (r: WrongRow): HTMLElement => {
    const btn = h('button', {
      id: `dash-wp-${r.rank}`, class: `dash-pt dash-pt-${r.rank}`, type: 'button', 'aria-pressed': 'false',
      'aria-label': t('dash.wrong.aria', { rank: r.rank, misses: missesText(r.wrong) }),
      onclick: () => { pick.pinned = togglePin(pick.pinned, r.id); paint(); },
      // A mouse shows the question while it is over the point; a keyboard while the point has the focus (the focus a click
      // leaves behind does not count, or unpinning would seem to do nothing)
      onpointerenter: (e: PointerEvent) => { if (e.pointerType === 'mouse') { pick.hover = r.id; paint(); } },
      onpointerleave: (e: PointerEvent) => { if (e.pointerType === 'mouse' && pick.hover === r.id) { pick.hover = null; paint(); } },
      onfocus: (e: FocusEvent) => { if ((e.target as HTMLElement).matches(':focus-visible')) { pick.hover = r.id; paint(); } },
      onblur: () => { if (pick.hover === r.id) { pick.hover = null; paint(); } },
    }) as HTMLButtonElement;
    buttons.set(r.id, btn);
    return h('li', null, btn);
  };
  const art = svg('svg', { class: 'dash-chalk', 'aria-hidden': 'true', focusable: 'false' });
  const board = h('div', { class: 'dash-board' },
    art, h('ol', { class: 'dash-pts', 'aria-label': t('dash.wrong.podium') }, ...podiumOrder(rows).map(place)));
  const draw = (w: number): void => {
    lay = boardLayout(w, labels);
    sizeBoard(board, art, lay);
    const lines = svg('g', {}, ...curvePaths(lay));
    const texts = svg('g', { class: 'dash-chalk-text' });
    for (const r of rows) {
      const p = lay.places.find((x) => x.rank === r.rank);
      if (!p) continue;
      const c = { x: p.x, y: p.y };
      lines.append(
        svg('path', { d: hachure(circlePoly(c, p.r - 0.6), 1.8, -41, rng(300 + p.rank), lay), class: 'dash-chalk-fill' }),
        ...strokes(sketchCircle(c, p.r, rng(100 + p.rank), lay), 'dash-chalk-dot'));
      texts.append(
        svg('text', { x: p.numX, y: p.numY, 'text-anchor': 'middle', class: p.rank === 1 ? 'dash-curve-n dash-curve-n1' : 'dash-curve-n' }, document.createTextNode(String(p.rank))),
        svg('text', { x: p.labelX, y: p.labelY, 'text-anchor': p.anchor, class: `dash-curve-t dash-t${p.labelPx}` }, document.createTextNode(labels[p.rank] ?? '')));
      const b = buttons.get(r.id);
      if (b) {
        b.style.left = `${p.target.x}px`;
        b.style.top = `${p.target.y}px`;
        b.style.width = `${p.target.w}px`;
        b.style.height = `${p.target.h}px`;
      }
    }
    lines.append(ring);
    art.replaceChildren(lines, texts);
    drawRing();
  };
  fitBoard(board, draw);
  const wrap = h('div', {
    class: 'dash-board-wrap',
    onkeydown: (e: KeyboardEvent) => { if (e.key === 'Escape' && pick.pinned) { pick.pinned = null; paint(); } },
  }, board, detail);
  paint();
  return wrap;
}

/** Loading: the empty board with a faint chalk curve, the same height as the real one */
export function boardSkeleton(): HTMLElement {
  const art = svg('svg', { class: 'dash-chalk', 'aria-hidden': 'true', focusable: 'false' });
  const board = h('div', { class: 'dash-board dash-board-skel' }, art);
  fitBoard(board, (w) => {
    const lay = boardLayout(w);
    sizeBoard(board, art, lay);
    art.replaceChildren(...curvePaths(lay));
  });
  return h('div', { class: 'dash-board-wrap', 'aria-hidden': 'true' }, board, h('div', { class: 'dash-pick' }));
}
