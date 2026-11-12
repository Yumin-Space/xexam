// The named media queries of the v1.0 layout (spec, "How to read it"). Every stylesheet repeats the PHONE query verbatim as
// `@media (max-width: 600px), (orientation: landscape) and (max-height: 500px) { /* PHONE */ … }`, so CSS and code agree on
// what a phone is. A query that is not here is asked for, not added on the side.

/** PHONE: portrait phones up to 600 px wide, and any short landscape screen (a phone turned sideways) */
export const PHONE_MQ = '(max-width: 600px), (orientation: landscape) and (max-height: 500px)';

/** A phone turned sideways: little height, so fixed bars and boxes get smaller */
export const SHORT_LANDSCAPE_MQ = '(orientation: landscape) and (max-height: 500px)';

/** The practice / exam action bar sits under the AI+ panel, pinned to the bottom (spec 2.6): up to 880 px, or short landscape */
export const BAR_MQ = '(max-width: 880px), (orientation: landscape) and (max-height: 500px)';

/** A touch screen as the main pointer */
export const TOUCH_MQ = '(pointer: coarse)';

/** Whether the media query matches now; false where `matchMedia` does not exist (node tests, very old browsers) */
export function matches(q: string): boolean {
  const mm = (globalThis as { matchMedia?: (query: string) => MediaQueryList }).matchMedia;
  return typeof mm === 'function' ? mm.call(globalThis, q).matches : false;
}
