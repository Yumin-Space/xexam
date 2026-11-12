// The pure rules of the AI summary overlay (v1.0 file name; the v0.9 popover rules left with the popover, spec 2.9): when a
// click on the backdrop closes it, which open moves focus to the title, where the overlay starts, and which element takes a
// lost focus. No DOM (it runs under `node --test`): the component reads rectangles and targets from the page and passes plain
// numbers and booleans in.

const clamp = (v: number, lo: number, hi: number): number => (hi < lo ? lo : Math.min(hi, Math.max(lo, v)));

/** The overlay closes on a click on its backdrop only when the pointer went down on the backdrop too (a drag out of the text does not) */
export const backdropCloses = (downOnDialog: boolean, clickOnDialog: boolean): boolean => downOnDialog && clickOnDialog;

/** Opened by the keyboard (a click event with no pointer, `detail` 0): focus goes to the title */
export const focusTitleOnOpen = (clickDetail: number): boolean => clickDetail === 0;

/**
 * Where the overlay's top edge goes: the bottom of the page's h1 plus 8 px (no h1 → 16 px), at least 16 px + the safe area,
 * at most 40 % of the window height.
 */
export function overlayTop(h1Bottom: number | null, vh: number, insetTop: number): number {
  const wanted = h1Bottom == null ? 16 : h1Bottom + 8;
  return clamp(wanted, 16 + insetTop, vh * 0.4);
}

/** The element that takes a lost focus: the first usable button when there is one, else the fallback (the window's title) */
export function pickFocusTarget<T>(button: T | null | undefined, fallback: T | null | undefined): T | null {
  return button ?? fallback ?? null;
}
