// Shared tooltips (v0.8): short explanations that used to be paragraphs (User management's bank controls, the AI+ box, the
// AI settings). A `span.has-tip` holds a control and, as its SIBLING (never inside a label, so the text does not become part
// of the control's name), a `span.tip[role=tooltip]` that the control names with aria-describedby.
//   · shown on hover (devices with a mouse) and on keyboard focus (`:focus-visible`, not `:focus-within`: a dialog focusing
//     its first box, or a mouse click on a checkbox, never pops a tip); a `button.tip-btn` (and the control of a `.has-tip.tip-tap`, e.g. the dark AI+ badge) also toggles its
//     tip on a tap, so touch phones can read it; a tap anywhere else closes it;
//   · placed ABOVE the control (below only when there is no room above), clamped to the window, `position: fixed` (a tip in
//     a <dialog> stays in its top layer), through el.style.left / top (CSSOM, allowed by the CSP `style-src 'self'`);
//   · hidden on Escape, on any click and on any scroll (until the next hover or focus), so no tip is ever left behind.
// v1.0: `holdTip(control, text)` gives any control (the icon buttons) a tip without a wrapper: hover / keyboard focus on
// desktop, tap-and-hold on touch and pen (the timing is domain/hold-tip.ts). One shared element holds the text.
import { holdIdle, holdStep, type HoldEvent, type HoldState } from '../domain/hold-tip.ts';
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

/** The side safe-area insets in px (read through the :root custom properties --sa-t / --sa-r / --sa-l of main.css) */
function insets(): { t: number; r: number; l: number } {
  const probe = document.createElement('div');
  probe.style.cssText = 'position:fixed;visibility:hidden;pointer-events:none;padding:var(--sa-t, 0px) var(--sa-r, 0px) 0 var(--sa-l, 0px)';
  document.body.append(probe);
  const cs = getComputedStyle(probe);
  const r = { t: parseFloat(cs.paddingTop) || 0, r: parseFloat(cs.paddingRight) || 0, l: parseFloat(cs.paddingLeft) || 0 };
  probe.remove();
  return r;
}

/** Puts the already displayed `tip` above `control` (below when there is no room above), inside the window */
function placeNear(tip: HTMLElement, control: HTMLElement): void {
  const tr = tip.getBoundingClientRect();
  const cr = control.getBoundingClientRect();
  const vw = document.documentElement.clientWidth || window.innerWidth;
  const vh = window.innerHeight;
  const ins = insets();
  const minTop = EDGE + ins.t;
  let top = cr.top - tr.height - GAP;
  if (top < minTop) top = Math.min(cr.bottom + GAP, vh - tr.height - EDGE);
  const minLeft = EDGE + ins.l;
  const left = Math.min(Math.max(cr.left, minLeft), vw - ins.r - tr.width - EDGE);
  tip.style.left = `${Math.round(Math.max(minLeft, left))}px`;
  tip.style.top = `${Math.round(Math.max(minTop, top))}px`;
}

/** Puts the tip of `wrap` above its control (below when there is no room), inside the window */
function place(wrap: HTMLElement): void {
  wrap.classList.remove('tip-hidden');
  const tip = wrap.querySelector<HTMLElement>(':scope > .tip');
  const control = Array.from(wrap.children).find((c) => !c.classList.contains('tip')) as HTMLElement | undefined;
  if (!tip || !control) return;
  tip.classList.add('tip-measure');
  placeNear(tip, control);
  tip.classList.remove('tip-measure');
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

// ---------- Tips for controls without a wrapper (v1.0): hover and keyboard focus on desktop, tap-and-hold on touch ----------

/** The text of each control that has a hold tip (calling `holdTip` again only changes the text) */
const holdTexts = new WeakMap<HTMLElement, string>();
let holdEl: HTMLElement | null = null;
let hold: HoldState = holdIdle();
let holdOwner: HTMLElement | null = null;
/** The control the shared tip is shown for (a blur of another control must not hide it) */
let shownFor: HTMLElement | null = null;
let holdTimer: number | undefined;
let hideTimer: number | undefined;
let holdGlobals = false;

/** The one shared tip element (aria-hidden: the control's own aria-label is its accessible name) */
function holdTipEl(): HTMLElement {
  if (!holdEl) holdEl = h('div', { class: 'hold-tip', 'aria-hidden': 'true' });
  return holdEl;
}

function showHoldTip(control: HTMLElement): void {
  const text = holdTexts.get(control);
  if (!text) return;
  window.clearTimeout(hideTimer);
  const el = holdTipEl();
  // Inside an open modal dialog the tip must live in the dialog's top layer, or the backdrop would hide it
  const host = control.closest('dialog[open]') ?? document.body;
  if (el.parentElement !== host) host.append(el);
  el.textContent = text;
  shownFor = control;
  el.classList.add('hold-tip-on');
  placeNear(el, control);
}

function hideHoldTip(afterMs = 0): void {
  window.clearTimeout(hideTimer);
  if (afterMs > 0) hideTimer = window.setTimeout(() => hideHoldTip(0), afterMs);
  else {
    shownFor = null;
    holdEl?.classList.remove('hold-tip-on');
  }
}

/** Feeds one event into the hold machine and applies its effects; returns true when the click must be swallowed */
function feedHold(control: HTMLElement, ev: HoldEvent): boolean {
  const r = holdStep(hold, ev, Date.now());
  hold = r.s;
  holdOwner = control;
  let swallow = false;
  for (const fx of r.fx) {
    if (fx.type === 'startTimer') {
      window.clearTimeout(holdTimer);
      holdTimer = window.setTimeout(() => {
        if (holdOwner) feedHold(holdOwner, { type: 'timer' });
      }, fx.ms);
    } else if (fx.type === 'clearTimer') window.clearTimeout(holdTimer);
    else if (fx.type === 'show') showHoldTip(control);
    else if (fx.type === 'hide') hideHoldTip(fx.afterMs);
    else swallow = true;
  }
  return swallow;
}

function installHoldGlobals(): void {
  if (holdGlobals) return;
  holdGlobals = true;
  window.addEventListener('scroll', () => {
    if (holdOwner) feedHold(holdOwner, { type: 'scroll' });
    hideHoldTip();
  }, true);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') hideHoldTip();
  }, true);
  window.addEventListener('resize', () => hideHoldTip());
}

/**
 * Gives `control` (an icon button) its tip: shown on mouse hover and keyboard focus, and on touch / pen by holding the finger
 * on it for 450 ms (the click that ends such a press is swallowed once). `text` is what the control is called.
 */
export function holdTip<T extends HTMLElement>(control: T, text: string): T {
  const first = !holdTexts.has(control);
  holdTexts.set(control, text);
  if (!first) return control;
  installHoldGlobals();
  const touchy = (e: PointerEvent): boolean => e.pointerType !== 'mouse';
  // The press belongs to this control, or to the control it replaced while the finger was down (a page that redraws its
  // buttons, e.g. AI Chat's messages, gives the new one the same id): the release and its click then count for the hold,
  // so a held button that was redrawn meanwhile never fires on release
  const mine = (): boolean => holdOwner === control
    || (holdOwner != null && !holdOwner.isConnected && holdOwner.id !== '' && holdOwner.id === control.id);
  control.addEventListener('pointerenter', (e) => {
    if (!touchy(e)) showHoldTip(control);
  });
  control.addEventListener('pointerleave', (e) => {
    if (!touchy(e) && shownFor === control) hideHoldTip();
  });
  control.addEventListener('focus', () => {
    if (control.matches(':focus-visible')) showHoldTip(control);
  });
  control.addEventListener('blur', () => {
    if (shownFor === control) hideHoldTip();
  });
  control.addEventListener('pointerdown', (e) => {
    if (touchy(e)) feedHold(control, { type: 'down', x: e.clientX, y: e.clientY });
    else hideHoldTip();
  });
  control.addEventListener('pointermove', (e) => {
    if (touchy(e) && mine()) feedHold(control, { type: 'move', x: e.clientX, y: e.clientY });
  });
  control.addEventListener('pointerup', (e) => {
    if (touchy(e) && mine()) feedHold(control, { type: 'up' });
  });
  control.addEventListener('pointercancel', (e) => {
    if (touchy(e) && mine()) feedHold(control, { type: 'cancel' });
  });
  // Capture phase at the target: runs before the control's own click handler, so a swallowed click never reaches it
  control.addEventListener('click', (e) => {
    // A keyboard click (detail 0) has no press before it: never part of a hold
    if (e.detail !== 0 && mine() && feedHold(control, { type: 'click' })) {
      e.stopImmediatePropagation();
      e.preventDefault();
    }
  }, true);
  control.addEventListener('contextmenu', (e) => e.preventDefault());
  return control;
}
