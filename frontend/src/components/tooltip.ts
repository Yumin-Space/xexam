// Shared tooltips (v0.8): short explanations that used to be paragraphs (User management's bank controls, the AI+ box, the
// AI settings). A `span.has-tip` holds a control and, as its SIBLING (never inside a label, so the text does not become part
// of the control's name), a `span.tip[role=tooltip]` that the control names with aria-describedby.
//   · shown on hover (devices with a mouse) and on keyboard focus (`:focus-visible`, not `:focus-within`: a dialog focusing
//     its first box, or a mouse click on a checkbox, never pops a tip); a `button.tip-btn` (and the control of a `.has-tip.tip-tap`, e.g. the dark AI+ badge) also toggles its
//     tip on a tap, so touch phones can read it; a tap anywhere else closes it;
//   · placed ABOVE the control (below only when there is no room above), clamped to the window, `position: fixed` (a tip in
//     a <dialog> stays in its top layer), through el.style.left / top (CSSOM, allowed by the CSP `style-src 'self'`);
//   · hidden on Escape, on any click and on any scroll (until the next hover or focus), so no tip is ever left behind.
import { h, type Child } from '../utils/dom.ts';
import { miniIcon } from './mini-icons.ts';

/** The tooltip element: `span.tip[role=tooltip][id]` */
export function tipEl(id: string, text: string): HTMLSpanElement {
  return h('span', { class: 'tip', role: 'tooltip', id }, text);
}

/** A control (or a label holding it) and its tooltip, side by side in a `span.has-tip` */
export function withTip(control: Child, tip: HTMLElement, cls = ''): HTMLSpanElement {
  return h('span', { class: 'has-tip' + (cls ? ' ' + cls : '') }, control, tip);
}

/** A small "i" button that explains something (its accessible name says what about; its tip says the rest) */
export function infoTip(id: string, label: string, text: string, cls = ''): { el: HTMLSpanElement; tip: HTMLSpanElement; button: HTMLButtonElement } {
  const tip = tipEl(id, text);
  const button = h('button', { type: 'button', class: 'tip-btn', 'aria-label': label, 'aria-describedby': id }, miniIcon('info'));
  return { el: withTip(button, tip, cls), tip, button };
}

const GAP = 6;
const EDGE = 4;

/** Puts the tip of `wrap` above its control (below when there is no room), inside the window */
function place(wrap: HTMLElement): void {
  wrap.classList.remove('tip-hidden');
  const tip = wrap.querySelector<HTMLElement>(':scope > .tip');
  const control = Array.from(wrap.children).find((c) => !c.classList.contains('tip')) as HTMLElement | undefined;
  if (!tip || !control) return;
  tip.classList.add('tip-measure');
  const tr = tip.getBoundingClientRect();
  tip.classList.remove('tip-measure');
  const cr = control.getBoundingClientRect();
  const vw = document.documentElement.clientWidth || window.innerWidth;
  const vh = window.innerHeight;
  let top = cr.top - tr.height - GAP;
  if (top < EDGE) top = Math.min(cr.bottom + GAP, vh - tr.height - EDGE);
  const left = Math.min(Math.max(cr.left, EDGE), vw - tr.width - EDGE);
  tip.style.left = `${Math.round(Math.max(EDGE, left))}px`;
  tip.style.top = `${Math.round(Math.max(EDGE, top))}px`;
}

function hideAll(except: Element | null = null): void {
  document.querySelectorAll<HTMLElement>('.has-tip').forEach((w) => {
    if (w === except) return;
    w.classList.remove('tip-open');
    w.classList.add('tip-hidden');
  });
}

let installed = false;
/** The `.has-tip` under the mouse (a new one counts as "mouseenter") */
let hovered: Element | null = null;

const wrapOf = (target: EventTarget | null): HTMLElement | null =>
  target instanceof Element ? target.closest<HTMLElement>('.has-tip') : null;

/** Installs the listeners of every tooltip (once; main.ts) */
export function installTooltips(): void {
  if (installed) return;
  installed = true;
  document.addEventListener('mouseover', (e) => {
    const wrap = wrapOf(e.target);
    if (wrap === hovered) return;
    hovered = wrap;
    if (wrap) place(wrap);
  });
  document.addEventListener('focusin', (e) => {
    const wrap = wrapOf(e.target);
    if (wrap) place(wrap);
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') hideAll();
  }, true);
  document.addEventListener('click', (e) => {
    const btn = e.target instanceof Element ? e.target.closest('.tip-btn, .tip-tap > :not(.tip)') : null;
    const own = btn ? wrapOf(btn) : null;
    hideAll(own);
    if (!own) return;
    const open = !own.classList.contains('tip-open');
    own.classList.toggle('tip-open', open);
    if (open) place(own);
  }, true);
  window.addEventListener('scroll', () => hideAll(), true);
  window.addEventListener('resize', () => hideAll());
}
