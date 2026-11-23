// Phone answer sheet as a left drawer (v2.7). The state lives here (module level) because the quiz page is rebuilt on most
// actions; a rebuilt drawer that already has `.open` does not animate. The pattern is the AI Chat history drawer's
// (pages/ai.ts): one pushed history entry so the Back gesture closes it, left swipe, Esc, Tab trap, focus rules.
// The DOM is the quiz page's own `aside.rail` (same DOM at every width); only under PHONE_MQ does it act as a drawer.
import { PHONE_MQ, matches } from '../utils/media.ts';

export const RAIL_ID = 'qz-rail';
export const BACKDROP_ID = 'qz-backdrop';
export const POS_BTN_ID = 'qz-pos';
const HTML_CLASS = 'qz-sheet-open';
const SHEET_ENTRY = { qzSheet: true } as const;

let open = false;
/** The drawer pushed one history entry */
let pushed = false;
/** A history.back() made here to drop that entry is under way: its popstate is not a user's back */
let popping = false;
let after: (() => void) | null = null;
let timer = 0;
let savedY = 0;
let pointerInput = true;

const rail = (): HTMLElement | null => document.getElementById(RAIL_ID);
const isSheetEntry = (state: unknown): boolean => typeof state === 'object' && state !== null && (state as { qzSheet?: unknown }).qzSheet === true;
const onQuiz = (): boolean => !!document.querySelector('#app .qz');
/** The hash decides while the route is not redrawn yet (listener order) */
const quizHash = (): boolean => /^#\/quiz(?:[/?]|$)/.test(location.hash);

/** The drawer is open (read by the keyboard shortcuts and auto-advance) */
export function sheetOpen(): boolean {
  return open && matches(PHONE_MQ);
}

/** The quiz screen is entered: the drawer starts closed */
export function resetSheet(): void {
  window.clearTimeout(timer);
  open = false;
  pushed = false;
  popping = false;
  after = null;
  lockPage(false);
}

/** Draws the state (of `scope`: a freshly built page that is not in the document yet, or the document): rail class + dialog attributes, backdrop, the trigger's aria-expanded, the page lock */
export function applySheet(scope: ParentNode = document): void {
  const phone = matches(PHONE_MQ);
  const on = open && phone;
  const el = scope.querySelector<HTMLElement>('#' + RAIL_ID);
  if (el) {
    el.classList.toggle('open', on);
    // dialog semantics only while it is a drawer (under PHONE); wide screens keep the plain landmark
    // and only while it is open: a closed drawer is inert (out of the tab order and hidden from assistive tech)
    if (on) {
      el.setAttribute('role', 'dialog');
      el.setAttribute('aria-modal', 'true');
      el.setAttribute('tabindex', '-1');
    } else {
      el.removeAttribute('role');
      el.removeAttribute('aria-modal');
      el.removeAttribute('tabindex');
    }
    el.toggleAttribute('inert', phone && !on);
  }
  const back = scope.querySelector<HTMLElement>('#' + BACKDROP_ID);
  if (back) back.hidden = !on;
  scope.querySelector('#' + POS_BTN_ID)?.setAttribute('aria-expanded', String(on));
  lockPage(on);
}

/** The page behind the drawer does not scroll; the scroll position is kept (iOS may jump when html overflow changes) */
function lockPage(on: boolean): void {
  const root = document.documentElement;
  if (on === root.classList.contains(HTML_CLASS)) return;
  if (on) {
    savedY = window.scrollY;
    root.classList.add(HTML_CLASS);
  } else {
    root.classList.remove(HTML_CLASS);
    if (Math.abs(window.scrollY - savedY) > 1) window.scrollTo(0, savedY);
  }
}

export function openSheet(): void {
  if (open || !matches(PHONE_MQ)) return;
  open = true;
  history.pushState(SHEET_ENTRY, '', location.hash);
  pushed = true;
  applySheet();
  // The focus goes to the drawer itself (no keyboard pops up); the current tile is brought into view inside it
  rail()?.focus({ preventScroll: true });
  rail()?.querySelector<HTMLElement>('.tile.cur')?.scrollIntoView({ block: 'nearest' });
}

/**
 * Closes the drawer; `then` runs once the pushed history entry is gone (a tile tap redraws the page, which must not race
 * the history.back()). A keyboard user's focus returns to the trigger; after a tap it is only blurred (Safari blue ring).
 */
export function closeSheet(then?: () => void, focusBack = true): void {
  const was = open;
  open = false;
  applySheet();
  if (was && focusBack) focusTrigger();
  if (!pushed) {
    then?.();
    return;
  }
  pushed = false;
  popping = true;
  after = then ?? null;
  // popstate normally finishes this; the timer covers a browser that never fires it
  timer = window.setTimeout(finishPop, 600);
  history.back();
}

export function toggleSheet(): void {
  if (open) closeSheet();
  else openSheet();
}

function finishPop(): void {
  window.clearTimeout(timer);
  if (!popping) return;
  popping = false;
  const run = after;
  after = null;
  run?.();
}

function focusTrigger(): void {
  const btn = document.getElementById(POS_BTN_ID);
  if (!pointerInput) {
    btn?.focus({ preventScroll: true });
    return;
  }
  const at = document.activeElement;
  const el = rail();
  if (at instanceof HTMLElement && el && (at === el || el.contains(at))) at.blur();
}

// ---- swipe left to close (passive listeners, put on the rail by the page) ----
let sx = 0;
let sy = 0;
let swiping = false;
export function onSheetTouchStart(e: TouchEvent): void {
  const t0 = e.touches.length === 1 ? e.touches[0] : undefined;
  swiping = !!t0;
  if (t0) {
    sx = t0.clientX;
    sy = t0.clientY;
  }
}
export function onSheetTouchEnd(e: TouchEvent): void {
  const t0 = swiping ? e.changedTouches[0] : undefined;
  swiping = false;
  if (!t0 || !open) return;
  const dx = t0.clientX - sx;
  const dy = t0.clientY - sy;
  if (dx < -60 && Math.abs(dx) > Math.abs(dy)) closeSheet();
}

function trapTab(e: KeyboardEvent): void {
  const el = rail();
  if (!el) return;
  const items = Array.from(el.querySelectorAll<HTMLElement>('button, [href], input, [tabindex]:not([tabindex="-1"])'))
    .filter((x) => !(x instanceof HTMLButtonElement && x.disabled) && !x.hidden && x.offsetParent !== null);
  if (items.length === 0) {
    e.preventDefault();
    return;
  }
  const first = items[0]!;
  const last = items[items.length - 1]!;
  const at = document.activeElement;
  if (e.shiftKey && (at === first || at === el)) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && at === last) {
    e.preventDefault();
    first.focus();
  } else if (!(at instanceof HTMLElement) || !el.contains(at)) {
    e.preventDefault();
    first.focus();
  }
}

/** Once at startup: Back gesture, Esc / Tab, leaving the quiz, crossing the PHONE line */
export function installSheetListeners(): void {
  document.addEventListener('pointerdown', () => { pointerInput = true; }, true);
  document.addEventListener('keydown', () => { pointerInput = false; }, true);
  window.addEventListener('popstate', () => {
    if (popping) {
      finishPop();
      return;
    }
    if (!quizHash()) {
      resetSheet();
      return;
    }
    // Forward reached the entry a closed drawer left behind: step back over it again
    if (!open && isSheetEntry(history.state)) {
      popping = true;
      after = null;
      timer = window.setTimeout(finishPop, 600);
      history.back();
      return;
    }
    if (open) {
      pushed = false;
      open = false;
      applySheet();
      focusTrigger();
    }
  });
  // The route may not be redrawn yet when this runs (listener order), so the new hash decides, not the DOM
  window.addEventListener('hashchange', () => {
    if (!quizHash()) resetSheet();
  });
  // A page swap without a hash change (session expiry draws the sign-in page): the drawer's lock and state go with the quiz
  if (typeof MutationObserver === 'function') {
    const app = document.getElementById('app');
    if (app) {
      new MutationObserver(() => {
        if ((open || document.documentElement.classList.contains(HTML_CLASS)) && !onQuiz()) resetSheet();
      }).observe(app, { childList: true });
    }
  }
  document.addEventListener('keydown', (e) => {
    if (!open || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === 'Escape') {
      if (document.querySelector('dialog[open]') || e.isComposing) return;
      e.preventDefault();
      closeSheet();
    } else if (e.key === 'Tab') {
      if (document.querySelector('dialog[open]')) return;
      trapTab(e);
    }
  });
  // The viewport leaves PHONE (resize / rotation) with the drawer open: it is dropped (and its history entry)
  if (typeof window.matchMedia === 'function') {
    window.matchMedia(PHONE_MQ).addEventListener('change', () => {
      if (open && !matches(PHONE_MQ)) {
        const had = pushed;
        resetSheet();
        if (had) {
          popping = true;
          timer = window.setTimeout(finishPop, 600);
          history.back();
        }
      }
      applySheet();
    });
  }
}
