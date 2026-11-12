import { toastLiftPx } from '../domain/viewport.ts';

let timer: number | undefined;

/** Toast bar at the bottom of the screen. ms defaults to 2.5 seconds (callers pass 3500 for a two-line text). */
export function toast(msg: string, ms = 2500): void {
  const t = document.getElementById('toast');
  if (!t) return;
  t.textContent = msg;
  // the pinned bar may have moved since the last measure (a short page scrolled): measure it again before showing
  refreshLift?.();
  t.classList.add('show');
  window.clearTimeout(timer);
  timer = window.setTimeout(() => t.classList.remove('show'), ms);
}

// ---------- Keeping toasts above a pinned bottom bar (SH-4) ----------

let stopLift: (() => void) | null = null;
let refreshLift: (() => void) | null = null;

/** The bottom safe-area inset in px (measured through a throw-away probe; env() cannot be read directly) */
function safeBottomPx(): number {
  const p = document.createElement('div');
  p.style.cssText = 'position:fixed;visibility:hidden;pointer-events:none;padding-bottom:env(safe-area-inset-bottom)';
  document.body.append(p);
  const v = parseFloat(getComputedStyle(p).paddingBottom) || 0;
  p.remove();
  return v;
}

function setLift(px: number): void {
  document.documentElement.style.setProperty('--toast-lift', `${px}px`);
}

/**
 * While `el` (a pinned bottom bar: the practice / exam action bar, the Home start bar, the AI Chat composer) is on screen,
 * toasts sit above it: `--toast-lift` on :root = the room it takes from the bottom of the window + 12 px, but only when the toast's
 * default place would touch it, and never so high that the toast leaves the screen; otherwise 24 px. `null` clears it. A page calls
 * this when it renders; the route change clears it again.
 */
export function liftToastAbove(el: HTMLElement | null): void {
  stopLift?.();
  stopLift = null;
  refreshLift = null;
  setLift(24);
  if (!el || typeof IntersectionObserver === 'undefined') return;
  let visible = true;
  // The bottom safe area changes only with the window (rotation): measured here and again on resize, not on every scroll
  let safeBottom = safeBottomPx();
  const apply = (): void => {
    if (!el.isConnected) return;
    const r = el.getBoundingClientRect();
    const t = document.getElementById('toast');
    const topH = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--top-h')) || 0;
    // The visible bottom: on iOS innerHeight includes the keyboard (--kb), and #toast's CSS adds --kb itself, so measure above it
    const kb = parseFloat(document.documentElement.style.getPropertyValue('--kb')) || 0;
    setLift(toastLiftPx(visible, r.top, r.bottom, window.innerHeight - kb, t?.offsetHeight ?? 40, topH, kb > 0 ? 0 : safeBottom));
  };
  const io = new IntersectionObserver((entries) => {
    const last = entries[entries.length - 1];
    if (last) visible = last.isIntersecting;
    apply();
  });
  io.observe(el);
  let ro: ResizeObserver | null = null;
  if (typeof ResizeObserver !== 'undefined') {
    ro = new ResizeObserver(apply);
    ro.observe(el);
  }
  // A sticky bar moves with the page until it is pinned: follow it (once per frame at most)
  let frame = 0;
  const onMove = (): void => {
    if (frame) return;
    frame = window.requestAnimationFrame(() => {
      frame = 0;
      apply();
    });
  };
  const onResize = (): void => {
    safeBottom = safeBottomPx();
    onMove();
  };
  window.addEventListener('scroll', onMove, { passive: true });
  window.addEventListener('resize', onResize);
  // The keyboard opening or closing changes --kb without a window resize (iOS): follow the visual viewport too
  window.visualViewport?.addEventListener('resize', onMove);
  apply();
  refreshLift = apply;
  stopLift = () => {
    io.disconnect();
    ro?.disconnect();
    window.removeEventListener('scroll', onMove);
    window.removeEventListener('resize', onResize);
    window.visualViewport?.removeEventListener('resize', onMove);
    if (frame) window.cancelAnimationFrame(frame);
  };
}

// Every route change clears the lift. This listener is registered when the module loads, before the router's, so a page that
// renders for the new route (and calls liftToastAbove again) always comes after it.
if (typeof window !== 'undefined') {
  window.addEventListener('hashchange', () => liftToastAbove(null));
}
