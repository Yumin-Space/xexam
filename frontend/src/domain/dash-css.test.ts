// v2.5 dashboard styles: the leaderboard name shrinks with an ellipsis and keeps the "You" tag, the 无法使用 tag stays a small
// pill on phones, and the dashboard's text colours pass WCAG AA (4.5:1) on their backgrounds in the light and the dark theme.
// The colours are read from the theme tokens of main.css. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { describe, it } from 'node:test';

const read = (rel: string): string => readFileSync(new URL(rel, import.meta.url), 'utf8');
const MAIN = read('../styles/main.css');
const DASH = read('../styles/dashboard.css');
const PAGE = read('../pages/dashboard.ts');

/** The declarations of the first rule whose selector is exactly `sel` (searched from `from`) */
function rule(css: string, sel: string, from = 0): Record<string, string> {
  const re = new RegExp(`(^|[}\\n])\\s*${sel.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'g');
  re.lastIndex = from;
  const m = re.exec(css);
  assert.ok(m, `rule ${sel}`);
  const out: Record<string, string> = {};
  for (const d of m[2]!.split(';')) {
    const i = d.indexOf(':');
    if (i > 0) out[d.slice(0, i).trim()] = d.slice(i + 1).trim();
  }
  return out;
}

/** The custom properties of a theme block */
function tokens(css: string, sel: string): Record<string, string> {
  const start = css.indexOf(`${sel} {`);
  assert.ok(start >= 0, sel);
  const body = css.slice(start, css.indexOf('}', start));
  const out: Record<string, string> = {};
  for (const m of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out[m[1]!] = m[2]!.trim();
  return out;
}

const LIGHT = tokens(MAIN, ':root');
const DARK = { ...LIGHT, ...tokens(MAIN, ':root[data-theme="dark"]') };

type Rgba = [number, number, number, number];

function color(v: string, theme: Record<string, string>): Rgba {
  const ref = /^var\((--[\w-]+)\)$/.exec(v.trim());
  if (ref) {
    const next = theme[ref[1]!];
    assert.ok(next, `token ${ref[1]}`);
    return color(next, theme);
  }
  // color-mix(in srgb, A P%, B): the podium tints (A and B are tokens or colours; the mix is opaque when both are)
  const mix = /^color-mix\(in srgb,\s*(.+?)\s+(\d+)%\s*,\s*(.+)\)$/.exec(v.trim());
  if (mix) {
    const a = color(mix[1]!, theme);
    const b = color(mix[3]!, theme);
    const k = Number(mix[2]) / 100;
    return [0, 1, 2, 3].map((i) => a[i]! * k + b[i]! * (1 - k)) as Rgba;
  }
  const hex = /^#([0-9a-f]{6})$/i.exec(v.trim());
  if (hex) {
    const n = parseInt(hex[1]!, 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 1];
  }
  const rgba = /^rgba\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*\)$/.exec(v.trim());
  assert.ok(rgba, `colour ${v}`);
  return [Number(rgba[1]), Number(rgba[2]), Number(rgba[3]), Number(rgba[4])];
}

const over = (top: Rgba, under: Rgba): Rgba =>
  [0, 1, 2].map((i) => top[i]! * top[3] + under[i]! * (1 - top[3])).concat(1) as Rgba;

function lum(c: Rgba): number {
  const ch = (x: number): number => { const s = x / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; };
  return 0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2]);
}

function contrast(fg: Rgba, bg: Rgba): number {
  const f = over(fg, bg);
  const [a, b] = [lum(f), lum(bg)].sort((x, y) => y - x);
  return (a! + 0.05) / (b! + 0.05);
}

const THEMES = { light: LIGHT, dark: DARK } as const;
const card = (th: Record<string, string>): Rgba => color('var(--card)', th);
const selfRow = (th: Record<string, string>): Rgba => over(color('var(--print-soft)', th), card(th));
const AA = 4.5;

describe('dashboard text contrast (WCAG AA)', () => {
  it('the #1 numeral on the card and on the own row', () => {
    const fg = rule(DASH, '.dash-no.first').color!;
    for (const [name, th] of Object.entries(THEMES)) {
      assert.ok(contrast(color(fg, th), card(th)) >= AA, `${name} card`);
      assert.ok(contrast(color(fg, th), selfRow(th)) >= AA, `${name} own row`);
    }
  });

  it('the hero sub-line: full --on-primary on --primary, no opacity', () => {
    const sub = rule(DASH, '.dash-hero-sub');
    assert.equal(sub.opacity, undefined);
    assert.equal(sub.color, undefined, 'inherits the hero colour');
    const hero = rule(DASH, '.dash-hero');
    for (const [name, th] of Object.entries(THEMES)) {
      assert.ok(contrast(color(hero.color!, th), color(hero.background!, th)) >= AA, name);
    }
  });

  it('the correct count on a plain row and on the own row', () => {
    const plain = rule(DASH, '.dash-good').color!;
    const selfLight = rule(DASH, '.dash-list > li.dash-self .dash-good').color!;
    const selfDark = rule(DASH, ':root[data-theme="dark"] .dash-list > li.dash-self .dash-good').color!;
    const media = DASH.indexOf('@media (prefers-color-scheme: dark)');
    assert.ok(media >= 0);
    assert.equal(rule(DASH, ':root:not([data-theme="light"]) .dash-list > li.dash-self .dash-good', media).color, selfDark);
    for (const [name, th] of Object.entries(THEMES)) assert.ok(contrast(color(plain, th), card(th)) >= AA, `${name} plain`);
    assert.ok(contrast(color(selfLight, LIGHT), selfRow(LIGHT)) >= AA, 'light own row');
    assert.ok(contrast(color(selfDark, DARK), selfRow(DARK)) >= AA, 'dark own row');
  });
});

describe('dashboard layout', () => {
  it('the AI card is a rounded soft surface with an amber thread, and its button never exceeds the card', () => {
    const card = rule(DASH, '.dash-ai');
    assert.equal(card.background, 'var(--card)');
    assert.match(card['border-radius']!, /^20px$/);
    assert.match(card['box-shadow']!, /inset 4px 0 0 var\(--ai-2\)/);
    const btn = rule(DASH, '.dash-ai .btn');
    assert.equal(btn['min-height'], '44px');
    assert.equal(btn['max-width'], '100%');
    // The reason beside a greyed button wraps instead of widening the card
    assert.equal(rule(DASH, '.dash-note')['min-width'], '0');
    assert.equal(rule(DASH, '.dash-ai-text')['white-space'], 'pre-line');
  });

  it('four small tiles go two by two on phones, one row wider; every tile keeps 88 px', () => {
    const phone = DASH.indexOf('@media (max-width: 480px)');
    assert.ok(phone >= 0);
    const grid = rule(DASH, '.dash-row[data-n="4"]', phone);
    assert.equal(grid.display, 'grid');
    assert.equal(grid['grid-template-columns'], 'repeat(2, minmax(0, 1fr))');
    assert.equal(rule(DASH, '.dash-row .dash-tile', phone)['min-height'], '88px');
    // The rule lives inside the phone query only: the base row stays a flex row
    assert.equal(rule(DASH, '.dash-row').display, 'flex');
    assert.match(PAGE, /'data-n': String\(row\.length\)/);
    assert.match(PAGE, /navAllowed\('banks', S\.user\)/);
  });
});

describe('chalk board (the most missed questions)', () => {
  const BOARD_T = read('../components/chalk-board.ts');
  const MEDIA = tokens(MAIN, ':root:not([data-theme="light"])');
  const CHALK = ['--board', '--board-line', '--chalk', '--chalk-dim', '--chalk-hi'];
  it('the board tokens exist in the light theme, the dark theme and the follow-system dark block (dark ones equal)', () => {
    for (const k of CHALK) {
      assert.ok(LIGHT[k], `light ${k}`);
      assert.ok(tokens(MAIN, ':root[data-theme="dark"]')[k], `dark ${k}`);
      assert.equal(MEDIA[k], tokens(MAIN, ':root[data-theme="dark"]')[k], `follow-system ${k}`);
    }
    // Owner: white board with black lines in the light theme, black board with white lines in the dark theme
    assert.ok(lum(color('var(--board)', THEMES.light)) > lum(color('var(--chalk)', THEMES.light)) + 0.5, 'light: dark lines on white');
    assert.ok(lum(color('var(--chalk)', THEMES.dark)) > lum(color('var(--board)', THEMES.dark)) + 0.5, 'dark: white lines on black');
  });

  it('every chalk text passes AA on the board, in both themes; the ring is a visible 3:1', () => {
    const num = rule(DASH, '.dash-curve-n, .dash-curve-t').fill!;
    const label = rule(DASH, '.dash-curve-t').fill!;
    assert.equal(num, 'var(--chalk)');
    assert.equal(label, 'var(--chalk-dim)');
    // No opacity on the texts (that would quietly lower the contrast)
    for (const sel of ['.dash-curve-n, .dash-curve-t', '.dash-curve-t', '.dash-curve-n1']) assert.equal(rule(DASH, sel).opacity, undefined, sel);
    for (const [name, th] of Object.entries(THEMES)) {
      const bg = color(rule(DASH, '.dash-board').background!, th);
      assert.ok(contrast(color(num, th), bg) >= AA, `${name} number`);
      assert.ok(contrast(color(label, th), bg) >= AA, `${name} label`);
      assert.ok(contrast(color('var(--chalk-hi)', th), bg) >= 3, `${name} ring`);
      // The hint and the meta line of the detail region sit on --fill under the board
      assert.ok(contrast(color('var(--ink3)', th), color('var(--fill)', th)) >= AA, `${name} hint`);
      assert.ok(contrast(color(rule(DASH, '.dash-pick-meta').color!, th), color('var(--fill)', th)) >= AA, `${name} meta`);
      assert.ok(contrast(color(rule(DASH, '.dash-ai-err').color!, th), card(th)) >= AA, `${name} error`);
      assert.ok(contrast(color(rule(DASH, '.dash-ai-meta').color!, th), card(th)) >= AA, `${name} card meta`);
    }
  });

  it('strokes: round caps, no fill, every line drawn as two strokes from the seeded sketch helper', () => {
    const path = rule(DASH, '.dash-chalk path');
    assert.equal(path.stroke, 'var(--chalk)');
    assert.equal(path['stroke-linecap'], 'round');
    assert.equal(path.fill, 'none');
    assert.match(BOARD_T, /ds\.map\(\(d, i\) => svg\('path', \{ d, class: `\$\{cls\} dash-s\$\{i \+ 1\}` \}\)\)/);
    assert.match(BOARD_T, /sketchCurve\(lay\.curve, rng\(\d+\)/);
    assert.equal(rule(DASH, '.dash-chalk path.dash-chalk-hi').stroke, 'var(--chalk-hi)');
  });

  it('the drawing is decoration only; each point is a real transparent button of at least 44 x 44 px', () => {
    assert.match(BOARD_T, /svg\('svg', \{ class: 'dash-chalk', 'aria-hidden': 'true', focusable: 'false' \}\)/);
    assert.equal(rule(DASH, '.dash-chalk')['pointer-events'], 'none');
    const pt = rule(DASH, '.dash-pt');
    assert.equal(pt.position, 'absolute');
    assert.equal(pt['min-width'], '44px');
    assert.equal(pt['min-height'], '44px');
    assert.equal(pt.background, 'transparent');
    assert.equal(rule(DASH, '.dash-board').position, 'relative');
    assert.match(rule(DASH, '.dash-pt:focus-visible').outline!, /var\(--chalk-hi\)/);
  });

  it('no motion on the board (nothing to switch off for reduced motion)', () => {
    for (const m of DASH.matchAll(/(\.dash-(?:board|chalk|pt|curve)[^{]*)\{([^}]*)\}/g)) {
      assert.doesNotMatch(m[2]!, /animation|transition/, m[1]!);
    }
  });

  it('the pick region keeps the whole stem: wrapped, never clipped', () => {
    const stem = rule(DASH, '.dash-pick-stem');
    assert.equal(stem['white-space'], 'pre-wrap');
    assert.equal(stem['overflow-wrap'], 'anywhere');
    assert.equal(stem['text-overflow'], undefined);
    assert.equal(stem['-webkit-line-clamp'], undefined);
    assert.ok(rule(DASH, '.dash-pick')['min-width'] === '0');
  });

  it('every point is a button with a spoken name and state, the region is polite and Escape clears a pin', () => {
    assert.match(BOARD_T, /h\('button', \{\s*id: `dash-wp-\$\{r\.rank\}`, class: `dash-pt dash-pt-\$\{r\.rank\}`, type: 'button', 'aria-pressed': 'false'/);
    assert.match(BOARD_T, /'aria-label': t\('dash\.wrong\.aria'/);
    assert.match(BOARD_T, /b\.setAttribute\('aria-pressed', String\(pick\.pinned === id\)\)/);
    // the detail is a region; a polite live line repeats the shown question (the layers themselves are aria-hidden)
    assert.match(BOARD_T, /role: 'region'/);
    assert.match(BOARD_T, /class: 'sr-only', 'aria-live': 'polite'/);
    // every question is drawn once as a layer of one grid cell, so hovering never changes the region's height
    assert.match(BOARD_T, /dash-pick-layer/);
    assert.equal(rule(DASH, '.dash-pick-layer')['grid-area'], '1 / 1');
    assert.match(BOARD_T, /e\.key === 'Escape' && pick\.pinned/);
    // Hover is for a mouse only; the keyboard counts only when the focus is visible
    assert.match(BOARD_T, /pointerType === 'mouse'/);
    assert.match(BOARD_T, /matches\(':focus-visible'\)/);
    // The board shows the stem in the default question language (the page resolves the rows)
    assert.match(PAGE, /wrongRows\(v\.data\.wrongTop\.items, S\.bank, S\.settings\.questionLang\)/);
    assert.match(PAGE, /chalkBoard\(rows, wrongStats\)/);
    // No medals, no gold / silver / bronze, no emoji (owner: medals look cheap)
    const boardCss = DASH.slice(DASH.indexOf('/* ---------- Chalk curve'), DASH.indexOf('/* The detail region'));
    for (const src of [BOARD_T, PAGE, boardCss]) {
      assert.doesNotMatch(src, /medal|gold|silver|bronze/i);
      assert.doesNotMatch(src, /\p{Extended_Pictographic}/u);
    }
  });

  it('wide screens (>= 900 px) use two equal columns, the phone column is untouched', () => {
    const wide = DASH.indexOf('@media (min-width: 900px)');
    assert.ok(wide >= 0);
    const page = rule(DASH, '.page-dash', wide);
    assert.equal(page['max-width'], '1080px');
    assert.equal(page['grid-template-columns'], 'repeat(2, minmax(0, 1fr))');
    assert.equal(rule(DASH, '.page-dash > *', wide)['grid-column'], '1 / -1');
    assert.equal(rule(DASH, '.page-dash > .dash-rank, .page-dash > .dash-wrong', wide)['grid-column'], 'auto');
    assert.equal(rule(DASH, '.dash-row, .dash-row:is([data-n="4"], [data-n="5"], [data-n="6"])', wide)['grid-auto-flow'], 'column');
    // the base column stays 560 px; the plane is hidden only on the narrowest phones
    assert.equal(rule(DASH, '.page-dash')['max-width'], '560px');
    assert.match(DASH, /@media \(max-width: 340px\) \{\s*html\[data-hand\] \.dash-doodle \{ display: none; \}/);
    assert.ok(!DASH.includes('.dash-skel') && !DASH.includes('.dash-bar'));
  });
});
