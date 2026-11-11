// The pure rules of the AI popover and the summary overlay (v0.9): where the popover goes (anchored, clamped to the window),
// which interactions close it, what clicking an opener does, and where the overlay starts. No DOM (it runs under
// `node --test`): the components read rectangles and targets from the page and pass plain numbers and booleans in.

export interface Rect {
  top: number;
  left: number;
  right: number;
  bottom: number;
}

export const EDGE = 8;
export const GAP = 6;
/** Less room than this below an opener puts the popover above it (when there is more room above) */
export const MIN_BELOW = 240;
/** The popover goes full width up to this window width */
export const NARROW = 600;
export const POP_WIDTH = 440;

export interface PlaceInput {
  opener: Rect;
  /** The popover's current size */
  pop: { width: number; height: number };
  vw: number;
  vh: number;
  /** env(safe-area-inset-*) in px */
  inset: { top: number; bottom: number };
  /** The opener is in the bottom action bar: open upwards */
  preferAbove: boolean;
}

export interface Placed {
  top: number;
  left: number;
  placement: 'below' | 'above';
  /** At most 600 px wide windows: left 8 / right 8, width auto */
  fullWidth: boolean;
}

const clamp = (v: number, lo: number, hi: number): number => (hi < lo ? lo : Math.min(hi, Math.max(lo, v)));

/** The popover's width for a window width (CSS: min(440px, 100vw - 16px)) */
export const popWidth = (vw: number): number => Math.min(POP_WIDTH, Math.max(0, vw - 2 * EDGE));

/**
 * Below the opener, right edges aligned; above when asked or when there is less than 240 px below and more above. The top is
 * clamped inside the window (so an opener scrolled out of view never takes the popover off screen) and the left too.
 */
export function placePopover(i: PlaceInput): Placed {
  const fullWidth = i.vw <= NARROW;
  const width = fullWidth ? Math.max(0, i.vw - 2 * EDGE) : i.pop.width;
  const left = fullWidth ? EDGE : clamp(i.opener.right - width, EDGE, i.vw - width - EDGE);
  const below = i.vh - i.opener.bottom;
  const above = i.opener.top;
  const goesAbove = i.preferAbove || (below < MIN_BELOW && above > below);
  const wanted = goesAbove ? i.opener.top - GAP - i.pop.height : i.opener.bottom + GAP;
  const lo = EDGE + i.inset.top;
  const hi = i.vh - i.pop.height - EDGE - i.inset.bottom;
  return { top: clamp(wanted, lo, hi), left, placement: goesAbove ? 'above' : 'below', fullWidth };
}

// ---------------------------------------------------------------- opener clicks and closing

export interface OpenState {
  kind: string;
  qid: string;
  source: string;
  anchorKey: string;
}

export interface OpenRequest {
  kind: string;
  qid: string;
  source: string;
  anchorKey: string;
}

/**
 * What a click on an opener does (spec 7.8): the open popover's own opener closes it; the other opener of the same
 * (question, kind, place) moves it there with the content kept; anything else shows another answer.
 */
export function togglePlan(open: OpenState | null, req: OpenRequest): 'open' | 'close' | 'move' | 'switch' {
  if (!open) return 'open';
  const same = open.qid === req.qid && open.kind === req.kind && open.source === req.source;
  if (!same) return 'switch';
  return open.anchorKey === req.anchorKey ? 'close' : 'move';
}

/**
 * A pointerdown closes the popover unless it is inside it, on one of its openers, or on a keep element (the card's language
 * switch: spec 7.4 / 7.5, a language switch keeps the popover open)
 */
export const outsidePointerCloses = (insidePop: boolean, onOpener: boolean, onKeep = false): boolean => !insidePop && !onOpener && !onKeep;

/** CSS selector of the elements that never close the popover: the language switch, anything marked `data-aip-keep`, the copy fallback textarea */
export const KEEP_SELECTOR = '.lang, [data-aip-keep], [data-aip-copy]';

/** After a page redraw the popover stays only while the opener it is anchored to still belongs to the same (qid, kind, source) */
export const openerStillMatches = (openerPair: string | null, curPair: string): boolean => openerPair == null || openerPair === curPair;

/** Focus leaving closes it only for a known element outside (a null target, e.g. focus lost by a redraw, keeps it open) */
export const focusOutCloses = (relatedKnown: boolean, relatedInPop: boolean, relatedIsOpener: boolean, relatedIsKeep = false): boolean =>
  relatedKnown && !relatedInPop && !relatedIsOpener && !relatedIsKeep;

/** Escape closes the popover whenever it is open and no modal dialog is (the dialog's own Esc handles that case) */
export const escapeClosesPopover = (popOpen: boolean, modalOpen: boolean): boolean => popOpen && !modalOpen;

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
