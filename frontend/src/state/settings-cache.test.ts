// The first paint before the sign-in is known: whose cached settings paint the page, and the sign-in page's language carried
// across the reload that follows a sign-in after someone else's sign-out.
import assert from 'node:assert/strict';
import { beforeEach, test } from 'node:test';
import { cacheSettings, cachedSettings, carryLanguage, firstPaintSettings, rememberUser } from './settings-cache.ts';
import { setStorageOwner } from './storage-keys.ts';

function memStorage(): { store: Storage; mem: Map<string, string> } {
  const mem = new Map<string, string>();
  const store = {
    getItem: (k: string) => (mem.has(k) ? (mem.get(k) as string) : null),
    setItem: (k: string, v: string) => void mem.set(k, String(v)),
    removeItem: (k: string) => void mem.delete(k),
    clear: () => mem.clear(),
    key: () => null,
    length: 0,
  } as Storage;
  return { store, mem };
}

const local = memStorage();
const session = memStorage();
(globalThis as unknown as { localStorage: Storage }).localStorage = local.store;
(globalThis as unknown as { sessionStorage: Storage }).sessionStorage = session.store;
// A Japanese browser
Object.defineProperty(globalThis, 'navigator', { value: { language: 'ja-JP', languages: ['ja-JP', 'en'] }, configurable: true });

const ZH = { theme: 'dark', uiLang: 'zh', autoAdvance: true, questionLang: 'zh' } as const;
const EN = { theme: 'light', uiLang: 'en', autoAdvance: false, questionLang: 'ja' } as const;

beforeEach(() => {
  local.mem.clear();
  session.mem.clear();
  setStorageOwner(null);
});

test('nobody signed in on this browser yet: the browser language', () => {
  assert.deepEqual(firstPaintSettings(), { theme: 'system', uiLang: 'ja', autoAdvance: false, questionLang: 'ja' });
});

test('the cache of the user who signed in last paints the page', () => {
  setStorageOwner(3);
  cacheSettings(ZH);
  rememberUser(3);
  setStorageOwner(null);
  assert.deepEqual(cachedSettings(), ZH);
  assert.deepEqual(firstPaintSettings(), ZH);
});

test('nothing is cached while nobody is signed in', () => {
  cacheSettings(ZH);
  assert.equal(local.mem.size, 0);
});

test('the language picked on the sign-in page survives the reload once, over the previous user cache', () => {
  setStorageOwner(3);
  cacheSettings(ZH);
  rememberUser(3);
  setStorageOwner(null);
  // bob (3, Chinese) signed out; the next person switched the sign-in page to English and signs in as a new account (9)
  rememberUser(9);
  carryLanguage('en');
  const first = firstPaintSettings();
  assert.equal(first.uiLang, 'en');
  assert.equal(first.theme, 'system', 'nothing else of the previous user');
  // read once
  assert.equal(firstPaintSettings().uiLang, 'ja');
});

test('the carried language changes only the language of the first paint', () => {
  setStorageOwner(9);
  cacheSettings(EN);
  rememberUser(9);
  setStorageOwner(null);
  carryLanguage('zh');
  assert.deepEqual(firstPaintSettings(), { ...EN, uiLang: 'zh' });
});

test('a broken note is ignored', () => {
  session.mem.set('xexam.pendingLang.v1', 'klingon');
  assert.equal(firstPaintSettings().uiLang, 'ja');
  assert.equal(session.mem.size, 0);
});

test('the cache of the version before users is read only until the first sign-in', () => {
  local.mem.set('xexam.settings.v1', JSON.stringify(ZH));
  assert.deepEqual(firstPaintSettings(), ZH);
  rememberUser(4);
  assert.equal(local.mem.has('xexam.settings.v1'), false);
});
