// A tiny DOM builder: all text is written into the page with createTextNode and HTML is never concatenated (content from Excel is untrusted).
// v1.0 adds the page-scroll helpers (scrollToY / isAppScroll), the on-screen keyboard variables (--vvh, --kb, html.kb-open),
// undoViewportPan and keepInView (spec SH-7, SH-13).
import { keyboardInset, revealDelta, shouldUndoPan } from '../domain/viewport.ts';
import { TOUCH_MQ, matches } from './media.ts';

export type Child = Node | string | number | false | null | undefined | Child[];
export type Props = Record<string, unknown> | null | undefined;

const PROPS = new Set(['checked', 'disabled', 'value', 'open']);

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, props?: Props, ...kids: Child[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props ?? {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = String(v);
    else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v as EventListener);
    else if (PROPS.has(k)) (el as unknown as Record<string, unknown>)[k] = v;
    // The style attribute (setAttribute) is blocked by the CSP style-src 'self'; writing through the CSSOM (cssText) is not restricted.
    else if (k === 'style') el.style.cssText = String(v);
    else el.setAttribute(k, v === true ? '' : String(v));
  }
  appendAll(el, kids);
  return el;
}

function appendAll(el: HTMLElement, kids: readonly Child[]): void {
  for (const kid of kids) {
    if (kid == null || kid === false) continue;
    if (Array.isArray(kid)) appendAll(el, kid);
    else el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
}

/** Split by line into multiple <p> elements (empty lines are dropped). */
export function paras(text: string | null | undefined, extra?: string, lang?: string): HTMLParagraphElement[] {
  return String(text ?? '')
    .split('\n')
    .filter((l) => l.trim())
    .map((l) => h('p', { lang: lang ?? 'ja', class: extra }, l.trim()));
}

const SVGNS = 'http://www.w3.org/2000/svg';

/** One brush stroke as a filled shape: from `a` (width wa) to `b` (width wb), the ends rounded by the outline's stroke */
function brush(a: [number, number], b: [number, number], wa: number, wb: number): string {
  const dx = b[0] - a[0], dy = b[1] - a[1], len = Math.hypot(dx, dy) || 1;
  const nx = -dy / len, ny = dx / len;
  const f = (v: number): string => v.toFixed(2);
  const p = (q: [number, number], w: number, side: number): string => `${f(q[0] + nx * w * side / 2)} ${f(q[1] + ny * w * side / 2)}`;
  // a slight belly in the middle of the stroke, as a brush presses harder there
  const m: [number, number] = [a[0] + dx * 0.45, a[1] + dy * 0.45];
  const wm = Math.max(wa, wb) * 1.12;
  return `M${p(a, wa, 1)} Q${p(m, wm, 1)} ${p(b, wb, 1)} L${p(b, wb, -1)} Q${p(m, wm, -1)} ${p(a, wa, -1)} Z`;
}

/**
 * Small correct / incorrect icons. Each carries two drawings: the plain tick / cross, and (v2.6, hand-written mode) a
 * teacher's red ink — for correct an oval ring drawn in one crayon stroke that runs past its start, for wrong a cross of two
 * brush strokes (the second starts with a round blob, both run out dry). The ink shapes are filled; a fine-noise filter
 * (components/hand-mode.ts, #xexam-crayon) roughens their edges. The stylesheet shows one of the two (html[data-hand]).
 */
export function svgMark(ok: boolean): SVGSVGElement {
  const s = document.createElementNS(SVGNS, 'svg');
  s.setAttribute('viewBox', '0 0 20 20');
  s.setAttribute('fill', 'none');
  s.setAttribute('stroke-width', '2.4');
  s.setAttribute('stroke-linecap', 'round');
  s.setAttribute('stroke-linejoin', 'round');
  s.setAttribute('aria-hidden', 'true');
  const el = (tag: string, attrs: Record<string, string>): SVGElement => {
    const e = document.createElementNS(SVGNS, tag);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    return e;
  };
  s.append(el('path', { d: ok ? 'M4 10.5l4 4 8-9' : 'M5 5l10 10M15 5L5 15', class: 'mk-plain' }));
  const ink = el('g', { class: 'mk-hand' });
  if (ok) {
    // the ring: a tilted oval, open at the lower right, its tail running down past the start
    ink.append(
      el('path', { d: 'M13.9 15.6C10.2 17.6 4.6 17.4 2.6 13.9C0.9 10.8 2.6 6.2 6.6 4.1C10.4 2.1 15.6 2.5 17.6 5.4C19.4 8.2 17.7 12.4 15.2 15.1C14 16.4 12.6 17.6 11.2 18.9', class: 'mk-ring' }),
      el('path', { d: 'M13.4 15.2C10 16.9 5.2 16.8 3.4 13.7C2 11 3.5 7 6.9 5.1', class: 'mk-ring mk-ring2' }));
  } else {
    // the first stroke from top left to bottom right, the second from a blob at the bottom left up to the top right
    ink.append(
      el('path', { d: brush([4.2, 3.4], [16.4, 16.8], 2.9, 1.5), class: 'mk-ink' }),
      el('path', { d: 'M3.6 3.0L6.4 6.6M4.9 2.6L7.6 5.6', class: 'mk-dry' }),
      el('circle', { cx: '4.6', cy: '15.8', r: '2', class: 'mk-ink' }),
      el('path', { d: brush([4.8, 15.6], [16.6, 3.6], 3.1, 1.1), class: 'mk-ink' }),
      el('path', { d: 'M14.6 4.9L17.6 2.2M15.6 6.2L18.2 3.9', class: 'mk-dry' }));
  }
  s.append(ink);
  return s;
}

/** An element that must exist; throws if it is not found, instead of returning null and crashing somewhere else. */
export function must<T extends Element>(sel: string, root: ParentNode = document): T {
  const el = root.querySelector<T>(sel);
  if (!el) throw new Error('Missing page element: ' + sel);
  return el;
}

interface Caret {
  value: string;
  start: number;
  end: number;
  dir: 'forward' | 'backward' | 'none' | undefined;
}

/** The caret (or selection) of a text field, or null for any other element and for input types without one (number, …) */
function caretOf(el: Element | null): Caret | null {
  if (!(el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) return null;
  try {
    const { selectionStart: start, selectionEnd: end } = el;
    if (start == null || end == null) return null;
    return { value: el.value, start, end, dir: el.selectionDirection ?? undefined };
  } catch {
    return null;
  }
}

/**
 * Runs a redraw and keeps the keyboard focus on the element with the same id (when it still exists). A text field drawn
 * anew with the same text also gets its caret or selection back, so typing goes on where it was (without this the caret
 * jumps to the end when a background load redraws the page).
 */
export function keepFocus(draw: () => void): void {
  const before = document.activeElement;
  const id = before instanceof HTMLElement && before !== document.body ? before.id : '';
  const caret = caretOf(before);
  draw();
  if (!id) return;
  const again = document.getElementById(id);
  if (!again) return;
  if (again !== document.activeElement) again.focus({ preventScroll: true });
  if (!caret || !(again instanceof HTMLInputElement || again instanceof HTMLTextAreaElement) || again.value !== caret.value) return;
  try {
    if (again.selectionStart !== caret.start || again.selectionEnd !== caret.end) again.setSelectionRange(caret.start, caret.end, caret.dir);
  } catch {
    // an input type without a caret
  }
}

// ---------- Page scrolling made by the app (SH-7) ----------

let appScrolling = false;
let appScrollStop: (() => void) | null = null;

/** Whether a scroll made by the app (`scrollToY`) is still under way; the shell top bar's scroll listener ignores it */
export function isAppScroll(): boolean {
  return appScrolling;
}

/**
 * Scrolls the page to `y`. Every programmatic page scroll in v1.0 goes through here: the flag of `isAppScroll` stays set
 * until `scrollend` (or 700 ms, for browsers without it and for a scroll that does not move), so the top bar does not
 * hide or show because of it. Smooth scrolling is dropped when the user asks for reduced motion.
 */
export function scrollToY(y: number, smooth: boolean): void {
  appScrollStop?.();
  appScrolling = true;
  let timer = 0;
  const stop = (): void => {
    window.clearTimeout(timer);
    window.removeEventListener('scrollend', stop);
    appScrolling = false;
    if (appScrollStop === stop) appScrollStop = null;
  };
  appScrollStop = stop;
  window.addEventListener('scrollend', stop);
  timer = window.setTimeout(stop, 700);
  const calm = typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  window.scrollTo({ top: y, behavior: smooth && !calm ? 'smooth' : 'auto' });
}

// ---------- On-screen keyboard (SH-13) ----------

let vvInstalled = false;

/** Fired on window after every update of --vvh / --kb / html.kb-open (only where there is a visualViewport) */
export const VIEWPORT_EVENT = 'xexam:viewport';

/**
 * Keeps `--vvh` (the visual viewport height in px) and `--kb` (the part of the layout viewport the keyboard covers, px)
 * on :root, and `html.kb-open` while `--kb` > 0, from one rAF-throttled visualViewport listener. Nothing is set where
 * there is no visualViewport. Runs once when this module loads; calling it again does nothing.
 */
export function installViewportVars(): void {
  if (vvInstalled || typeof window === 'undefined' || typeof document === 'undefined') return;
  const vv = window.visualViewport;
  if (!vv) return;
  vvInstalled = true;
  const root = document.documentElement;
  let frame = 0;
  const update = (): void => {
    frame = 0;
    const kb = keyboardInset(window.innerHeight, vv.height, vv.offsetTop);
    root.style.setProperty('--vvh', `${vv.height}px`);
    root.style.setProperty('--kb', `${kb}px`);
    root.classList.toggle('kb-open', kb > 0);
    // Tell the page the variables are applied (also after the 350 ms settle pass), so measuring code never reads stale values
    window.dispatchEvent(new Event(VIEWPORT_EVENT));
  };
  const queue = (): void => {
    if (!frame) frame = window.requestAnimationFrame(update);
  };
  // v1.2.1: with interactive-widget=resizes-content (iOS 26 Safari honours it too) the keyboard shrinks the LAYOUT viewport
  // as well, and the visualViewport resize can come before window.innerHeight has changed. Read only on that event, --kb
  // then held the whole keyboard height on top of the already shorter layout: the AI Chat composer was lifted twice, out
  // of the screen above the top bar, until the next tap. Re-read on every layout resize and once more after the keyboard
  // animation, so --kb always ends from settled values (0 when the layout itself shrank).
  let settle = 0;
  const queueSettled = (): void => {
    queue();
    window.clearTimeout(settle);
    settle = window.setTimeout(queue, 350);
  };
  vv.addEventListener('resize', queueSettled);
  vv.addEventListener('scroll', queue);
  window.addEventListener('resize', queueSettled);
  document.addEventListener('focusin', queueSettled);
  document.addEventListener('focusout', queueSettled);
  update();
}

/**
 * Undoes Safari's pan when a locked document (AI Chat, `html.no-doc-scroll`) was moved to bring a focused field into view:
 * back to the top, through scrollToY so the top bar's listener ignores it. Used by AI Chat only.
 */
export function undoViewportPan(): void {
  const locked = document.documentElement.classList.contains('no-doc-scroll');
  if (shouldUndoPan(locked, window.scrollY, window.visualViewport?.offsetTop ?? 0)) scrollToY(0, false);
}

/**
 * Keeps a text field (and its submit control) inside the visual viewport while the keyboard is up: on `focus` on touch,
 * again 300 ms later (the keyboard animates) and on every visualViewport `resize` while the field has the focus. A field in
 * a dialog scrolls the dialog's body; any other field scrolls the page (through scrollToY). Not for the AI Chat composer,
 * which never scrolls the page (CH-13).
 */
export function keepInView(field: HTMLElement, submit?: HTMLElement | null): void {
  const reveal = (): void => {
    if (!field.isConnected) return;
    const vv = window.visualViewport;
    const viewTop = vv ? vv.offsetTop : 0;
    const viewBottom = vv ? vv.offsetTop + vv.height : window.innerHeight;
    const f = field.getBoundingClientRect();
    const s = submit?.isConnected ? submit.getBoundingClientRect() : null;
    const top = s ? Math.min(f.top, s.top) : f.top;
    const bottom = s ? Math.max(f.bottom, s.bottom) : f.bottom;
    const delta = revealDelta(top, bottom, viewTop, viewBottom);
    if (delta === 0) return;
    if (field.closest('dialog')) field.scrollIntoView({ block: 'nearest' });
    else scrollToY(window.scrollY + delta, false);
  };
  let timer = 0;
  field.addEventListener('focus', () => {
    if (!matches(TOUCH_MQ)) return;
    reveal();
    window.clearTimeout(timer);
    timer = window.setTimeout(reveal, 300);
    window.visualViewport?.addEventListener('resize', reveal);
  });
  field.addEventListener('blur', () => {
    window.clearTimeout(timer);
    window.visualViewport?.removeEventListener('resize', reveal);
  });
}

installViewportVars();
