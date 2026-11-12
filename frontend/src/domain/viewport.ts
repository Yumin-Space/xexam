// Pure helpers for the on-screen keyboard and the toast lift (spec SH-4, SH-13). The DOM wiring is in utils/dom.ts and
// utils/toast.ts; everything here is plain arithmetic, so it runs under node.

/**
 * The part of the layout viewport that the on-screen keyboard covers, in px (never negative).
 * iOS keeps `innerHeight` and shrinks the visual viewport, so the keyboard shows as the difference; the visual viewport may
 * also be panned (`offsetTop`), which moves the covered strip, not its size. A pinch-zoomed viewport is smaller than the
 * window too, but then `offsetTop` plus `height` still reach the window's bottom edge or beyond, so the result is 0.
 * Android Chrome with `interactive-widget=resizes-content` shrinks `innerHeight` as well, so it gives 0.
 */
export function keyboardInset(innerHeight: number, vvHeight: number, vvOffsetTop: number): number {
  const v = Math.round(innerHeight - vvHeight - vvOffsetTop);
  return Number.isFinite(v) && v > 0 ? v : 0;
}

/**
 * How far the page must scroll (positive = down) so that the box [top, bottom] (client coordinates, for example a field and
 * its submit button) sits inside the visible band [viewTop, viewBottom] with `margin` px to spare. A box taller than the band
 * aligns its top (the field matters more than the button). 0 when it is inside already.
 */
export function revealDelta(top: number, bottom: number, viewTop: number, viewBottom: number, margin = 8): number {
  const room = viewBottom - viewTop - 2 * margin;
  if (bottom - top > room) return Math.round(top - (viewTop + margin));
  if (top < viewTop + margin) return Math.round(top - (viewTop + margin));
  if (bottom > viewBottom - margin) return Math.round(bottom - (viewBottom - margin));
  return 0;
}

/** Whether `undoViewportPan` must act: the document is locked (AI Chat) and the page or the visual viewport was moved */
export function shouldUndoPan(locked: boolean, scrollY: number, vvOffsetTop: number): boolean {
  return locked && (scrollY !== 0 || vvOffsetTop !== 0);
}

/** The toast's distance from the bottom edge (px, before the safe area). 24 (its default place) unless the bar is on screen
 *  and would touch the toast's default box; then just above the bar, but never so high that the toast leaves the screen
 *  (it stays below the top bar, `topH`). Client coordinates; `safeBottom` is the bottom safe-area inset. */
export function toastLiftPx(
  visible: boolean, barTop: number, barBottom: number, viewH: number, toastH: number, topH: number, safeBottom = 0,
): number {
  if (!visible || barBottom <= viewH - 24 - safeBottom - toastH) return 24;
  const above = viewH - barTop + 12;
  const maxLift = viewH - toastH - topH - 8 - safeBottom;
  return Math.max(24, Math.round(Math.min(above, maxLift)));
}
