// When the slim phone top bar hides (spec SH-7). No DOM, so it runs under `node --test`; components/shell.ts feeds the
// scroll position in and paints the result.
//   · Scrolling down by more than STEP px after the first START px hides the bar.
//   · Any upward scroll of STEP px shows it again; reaching the very top shows it too.
//   · Rubber-band positions (above the top, below the end) change nothing: iOS overshoots there and would flicker the bar.

/** The first part of the page, in px, where the bar never hides */
export const BAR_START = 56;
/** How far the page has to move in one direction before the bar follows, in px */
export const BAR_STEP = 8;

export interface BarState {
  hidden: boolean;
  /** The last scroll position seen */
  lastY: number;
  /** The direction of the current movement: 1 down, -1 up, 0 none yet */
  dir: -1 | 0 | 1;
  /** Where the current movement started */
  origin: number;
}

export function initialBar(y = 0): BarState {
  return { hidden: false, lastY: y, dir: 0, origin: y };
}

/** The bar after the page moved to `y`; `max` is the largest scroll position (`scrollHeight - innerHeight`) */
export function barStep(s: BarState, y: number, max: number): BarState {
  if (y < 0 || y > Math.max(0, max)) return s;
  if (y <= 0) return { hidden: false, lastY: y, dir: 0, origin: y };
  if (y === s.lastY) return s;
  const dir = y > s.lastY ? 1 : -1;
  const origin = dir === s.dir ? s.origin : s.lastY;
  let hidden = s.hidden;
  if (dir === 1 && !hidden && y > BAR_START && y - origin > BAR_STEP) hidden = true;
  if (dir === -1 && hidden && origin - y > BAR_STEP) hidden = false;
  return { hidden, lastY: y, dir, origin };
}
