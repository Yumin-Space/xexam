// The browser bar colour (the `theme-color` metas of index.html, spec SH-6). index.html has two: one for a light system, one
// for a dark one (`media`). A forced theme must set both to its own colour, or the dark meta would win on a dark system
// while the page is forced light. No DOM, so it runs under `node --test`.

export const LIGHT_BAR = '#F6F8FA';
export const DARK_BAR = '#0D1117';

export type ThemeChoice = 'system' | 'light' | 'dark';

/**
 * The colour for one `theme-color` meta. `media` is its media attribute: "follow system" gives each meta its own colour back
 * (the light one light, the one for a dark system dark); a meta without media follows the current system theme instead.
 */
export function barColor(theme: ThemeChoice, media: string | null, systemDark: boolean): string {
  if (theme === 'dark') return DARK_BAR;
  if (theme === 'light') return LIGHT_BAR;
  if (media && /dark/.test(media)) return DARK_BAR;
  if (media && /light/.test(media)) return LIGHT_BAR;
  return systemDark ? DARK_BAR : LIGHT_BAR;
}
