// Consistency of the UI catalogs (src/i18n/catalog/*.ts), beyond what TypeScript checks:
// zh / ja define exactly the English keys (English singular `_one` keys are optional there), use the same {placeholders},
// have no empty texts, keys carry their area prefix, no key is defined twice, and every catalog is registered in index.ts.
// Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

type Dict = Record<string, string>;
interface CatalogShape {
  en: Dict;
  zh: Dict;
  ja: Dict;
}

const LANGS = ['en', 'zh', 'ja'] as const;
const CATALOG_DIR = new URL('./catalog/', import.meta.url);
const INDEX_FILE = new URL('./index.ts', import.meta.url);
/** The shared catalog uses several prefixes (app., nav., common., err., kind., mode., act., …); every other catalog uses its file name. */
const SHARED = 'common';

const names = readdirSync(fileURLToPath(CATALOG_DIR))
  .filter((f) => f.endsWith('.ts') && !f.endsWith('.test.ts'))
  .map((f) => f.slice(0, -'.ts'.length))
  .sort();

const catalogs: Array<{ name: string; c: CatalogShape }> = [];
for (const name of names) {
  const mod = (await import(new URL(`${name}.ts`, CATALOG_DIR).href)) as { default: CatalogShape };
  catalogs.push({ name, c: mod.default });
}

const isSingular = (key: string): boolean => key.endsWith('_one');

function placeholders(s: string): string[] {
  return [...new Set([...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1] ?? ''))].sort();
}

test('catalog files are found and each is registered in index.ts', () => {
  assert.ok(names.includes(SHARED), `catalog/${SHARED}.ts not found`);
  assert.ok(names.length >= 2, `only ${names.length} catalog files found`);
  const index = readFileSync(fileURLToPath(INDEX_FILE), 'utf8');
  const all = /const ALL = \[([^\]]*)\]/.exec(index)?.[1] ?? '';
  const registered = all.split(',').map((s) => s.trim()).filter(Boolean);
  for (const name of names) {
    const imp = new RegExp(`import (\\w+) from '\\./catalog/${name}\\.ts';`).exec(index);
    assert.ok(imp, `index.ts does not import catalog/${name}.ts`);
    assert.ok(registered.includes(imp[1] ?? ''), `catalog/${name}.ts is imported but not listed in ALL in index.ts`);
    assert.match(index, new RegExp(`keyof typeof ${imp[1] ?? ''}\\.en`), `catalog/${name}.ts is missing from the MsgKey type in index.ts`);
  }
});

test('zh and ja define exactly the English keys (English singular _one keys are optional)', () => {
  const failures: string[] = [];
  for (const { name, c } of catalogs) {
    const en = Object.keys(c.en);
    for (const key of en.filter(isSingular)) {
      if (!(key.slice(0, -'_one'.length) in c.en)) failures.push(`${name}: en ${key} has no plural key ${key.slice(0, -'_one'.length)}`);
    }
    for (const lang of ['zh', 'ja'] as const) {
      const have = new Set(Object.keys(c[lang]));
      for (const key of en) if (!isSingular(key) && !have.has(key)) failures.push(`${name}: ${lang} is missing ${key}`);
      for (const key of have) if (!(key in c.en)) failures.push(`${name}: ${lang} has ${key}, which English does not have`);
    }
  }
  assert.deepEqual(failures, []);
});

test('every {placeholder} appears in all languages, and no text is empty or has a stray brace', () => {
  const failures: string[] = [];
  for (const { name, c } of catalogs) {
    for (const lang of LANGS) {
      for (const [key, value] of Object.entries(c[lang])) {
        if (typeof value !== 'string' || value.trim() === '') {
          failures.push(`${name}: ${lang} ${key} is empty`);
          continue;
        }
        if (/[{}]/.test(value.replace(/\{\w+\}/g, ''))) failures.push(`${name}: ${lang} ${key} has a brace that is not a {placeholder}: ${JSON.stringify(value)}`);
        if (lang === 'en') continue;
        const en = c.en[key];
        if (en === undefined) continue; // reported by the key test
        const want = placeholders(en);
        const got = placeholders(value);
        if (want.join() !== got.join()) failures.push(`${name}: ${lang} ${key} uses {${got.join('} {')}} but English uses {${want.join('} {')}}`);
      }
    }
  }
  assert.deepEqual(failures, []);
});

test('keys carry their area prefix and are defined in one catalog only', () => {
  const failures: string[] = [];
  const owner = new Map<string, string>();
  for (const { name, c } of catalogs) {
    for (const key of Object.keys(c.en)) {
      if (name !== SHARED && !key.startsWith(`${name}.`)) failures.push(`${name}: ${key} does not start with "${name}."`);
      const first = owner.get(key);
      if (first) failures.push(`${key} is defined in both ${first}.ts and ${name}.ts`);
      else owner.set(key, name);
    }
  }
  assert.deepEqual(failures, []);
});
