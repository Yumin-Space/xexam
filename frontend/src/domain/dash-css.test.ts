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
  it('a long name gets an ellipsis in its own shrinking box and the "You" tag stays whole', () => {
    const t = rule(DASH, '.dash-name-t');
    assert.equal(t['min-width'], '0');
    assert.equal(t.overflow, 'hidden');
    assert.equal(t['text-overflow'], 'ellipsis');
    assert.equal(t['white-space'], 'nowrap');
    // text-overflow does nothing on the flex row itself
    assert.equal(rule(DASH, '.dash-name')['text-overflow'], undefined);
    assert.equal(rule(DASH, '.dash-you').flex, 'none');
    assert.match(PAGE, /h\('span', \{ class: 'dash-name-t' \}, e\.name\)/);
  });

  it('the 无法使用 tag beside the greyed button: no extra margin, and a small pill on phones', () => {
    assert.equal(rule(DASH, '.dash-ai .aip-grp-inline > .aip-na')['margin-left'], '0');
    // the section start-aligns its rows, so the pill and the button keep their own width on phones too
    assert.equal(rule(DASH, '.dash-ai')['justify-items'], 'start');
    const phone = DASH.indexOf('@media (max-width: 480px)');
    assert.ok(phone >= 0);
    const btn = rule(DASH, '.dash-ai .btn', phone);
    assert.equal(btn.width, undefined);
    assert.equal(btn['max-width'], '100%');
  });
});
