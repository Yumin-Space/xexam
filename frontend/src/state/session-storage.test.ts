// Defensive tests for reading the saved exam: bad data is only dropped or corrected, never throws.
import assert from 'node:assert/strict';
import { beforeEach, test } from 'node:test';
import { adoptLegacySession, clearSaved, loadSaved, saveSession } from './session-storage.ts';
import { setStorageOwner } from './storage-keys.ts';
import type { Bank, Question } from '../types/api.ts';

// A minimal stand-in for localStorage
const mem = new Map<string, string>();
(globalThis as unknown as { localStorage: Storage }).localStorage = {
  getItem: (k: string) => (mem.has(k) ? (mem.get(k) as string) : null),
  setItem: (k: string, v: string) => void mem.set(k, String(v)),
  removeItem: (k: string) => void mem.delete(k),
  clear: () => mem.clear(),
  key: () => null,
  length: 0,
} as Storage;

/** The saved exam of user 7, the user signed in for most tests */
const KEY = 'xexam.u7.session.v1';
const LEGACY = 'xexam.session.v1';
const q = (id: string, kind: Question['kind'], extra: Partial<Question> = {}): Question => ({
  kind, tag: '', stem: id, options: [{ key: 'A', text: 'a' }, { key: 'B', text: 'b' }, { key: 'C', text: 'c' }],
  items: [], answer: ['A'], translation: '', zh: null, explanation: '', mnemonic: '', note: '', id, sheet: 'S', row: 1, bank: 1, ...extra,
});
const hot = q('h', 'hotspot', { options: [], items: [{ text: 'x', choices: ['はい', 'いいえ'] }, { text: 'y', choices: ['はい', 'いいえ'] }], answer: [0, 1] });
const bank = { file: '', mtime: '', error: '', sheets: [], questions: [q('s', 'single'), q('m', 'multi'), hot] } as Bank;

beforeEach(() => {
  mem.clear();
  setStorageOwner(7);
});
const put = (v: unknown): void => void mem.set(KEY, typeof v === 'string' ? v : JSON.stringify(v));

test('missing / not JSON / not an object / no ids: returns null, does not throw', () => {
  assert.equal(loadSaved(bank), null);
  for (const raw of ['not json', 'null', '5', '[]', '"x"', '{}', '{"ids": "s"}']) {
    put(raw);
    assert.equal(loadSaved(bank), null, raw);
  }
  assert.equal(loadSaved(null), null);
});

test('none of the ids are in the question bank: returns null', () => {
  put({ ids: ['gone1', 'gone2'] });
  assert.equal(loadSaved(bank), null);
});

test('normal round trip: reads back exactly what was saved', () => {
  saveSession({ qs: bank.questions, answers: { s: ['B'], h: [0, null] }, flags: new Set(['m']), idx: 1, elapsedSeconds: 75.9, mode: 'weak', sheets: ['S'], kind: 'exam', checked: new Set() });
  const r = loadSaved(bank);
  assert.ok(r);
  assert.equal(r.qs.length, 3);
  assert.deepEqual(r.answers.s, ['B']);
  assert.deepEqual(r.answers.h, [0, null]);
  assert.deepEqual([...r.flags], ['m']);
  assert.equal(r.idx, 1);
  assert.equal(r.elapsed, 75);
  assert.equal(r.mode, 'weak');
  assert.equal(r.answered, 1, 'a hotspot question answered only halfway does not count as answered');
  assert.equal(r.dropped, 0);
});

test('some questions are no longer in the question bank: skipped and counted', () => {
  put({ ids: ['s', 'gone', 'm'], answers: {}, idx: 0, elapsed: 0, mode: 'all', sheets: [] });
  const r = loadSaved(bank);
  assert.ok(r);
  assert.deepEqual(r.qs.map((x) => x.id), ['s', 'm']);
  assert.equal(r.dropped, 1);
});

test('answer refers to options that no longer exist: only the existing ones are kept', () => {
  put({ ids: ['m'], answers: { m: ['A', 'Z', 'C', 7, null] } });
  const r = loadSaved(bank);
  assert.ok(r);
  assert.deepEqual(r.answers.m, ['A', 'C']);
});

test('single choice saved with several / duplicate keys: normalized to one', () => {
  put({ ids: ['s', 'm'], answers: { s: ['A', 'B'], m: ['A', 'A', 'C'] } });
  const r = loadSaved(bank);
  assert.ok(r);
  assert.deepEqual(r.answers.s, ['A']);
  assert.deepEqual(r.answers.m, ['A', 'C']);
});

test('hotspot answer too long / too short / out of range / non-integer: truncated, padded with null, set to null', () => {
  put({ ids: ['h'], answers: { h: [0, 1, 1, 1] } });
  assert.deepEqual(loadSaved(bank)?.answers.h, [0, 1]);
  put({ ids: ['h'], answers: { h: [1] } });
  assert.deepEqual(loadSaved(bank)?.answers.h, [1, null]);
  put({ ids: ['h'], answers: { h: [5, -1] } });
  assert.equal(loadSaved(bank)?.answers.h, undefined, 'all out of range: treated as unanswered');
  put({ ids: ['h'], answers: { h: [0.5, '1'] } });
  assert.equal(loadSaved(bank)?.answers.h, undefined);
});

test('bad values of flags / idx / elapsed / mode / sheets are corrected', () => {
  put({ ids: ['s', 'm'], flags: 'abc', idx: 999, elapsed: 'x', mode: 'hacked', sheets: 5 });
  let r = loadSaved(bank);
  assert.ok(r);
  assert.equal(r.flags.size, 0);
  assert.equal(r.idx, 1, 'an out-of-range idx is clamped to the last question');
  assert.equal(r.elapsed, 0);
  assert.equal(r.mode, 'all');
  assert.deepEqual(r.sheets, []);
  put({ ids: ['s', 'm'], idx: -5, elapsed: -30, flags: ['s', 'nope', 3] });
  r = loadSaved(bank);
  assert.ok(r);
  assert.equal(r.idx, 0);
  assert.equal(r.elapsed, 0);
  assert.deepEqual([...r.flags], ['s']);
});

test('clearSaved clears the saved exam; keys from the old version are not read', () => {
  saveSession({ qs: bank.questions, answers: {}, flags: new Set(), idx: 0, elapsedSeconds: 0, mode: 'all', sheets: [], kind: 'exam', checked: new Set() });
  assert.ok(loadSaved(bank));
  clearSaved();
  assert.equal(loadSaved(bank), null);
  mem.set('someother.session.v1', JSON.stringify({ ids: ['s'] }));
  assert.equal(loadSaved(bank), null);
});

test('each user has their own saved exam; nobody signed in reads and writes nothing', () => {
  saveSession({ qs: bank.questions, answers: { s: ['B'] }, flags: new Set(), idx: 0, elapsedSeconds: 5, mode: 'all', sheets: [], kind: 'exam', checked: new Set() });
  assert.ok(mem.has(KEY));
  setStorageOwner(8);
  assert.equal(loadSaved(bank), null, 'another user does not see it');
  clearSaved();
  assert.ok(mem.has(KEY), "another user's clearSaved leaves it alone");
  setStorageOwner(null);
  assert.equal(loadSaved(bank), null);
  saveSession({ qs: bank.questions, answers: {}, flags: new Set(), idx: 0, elapsedSeconds: 0, mode: 'all', sheets: [], kind: 'exam', checked: new Set() });
  assert.deepEqual([...mem.keys()], [KEY], 'nothing is written without a user');
  setStorageOwner(7);
  assert.deepEqual(loadSaved(bank)?.answers.s, ['B']);
});

test('the saved exam of the version before users goes to the first administrator only', () => {
  const old = JSON.stringify({ ids: ['s'], answers: { s: ['C'] } });
  mem.set(LEGACY, old);
  assert.equal(loadSaved(bank), null, 'the old key is not read directly');
  adoptLegacySession(7, false);
  assert.equal(mem.get(LEGACY), old, 'a plain user does not take it');
  assert.equal(loadSaved(bank), null);
  adoptLegacySession(7, true);
  assert.equal(mem.has(LEGACY), false);
  assert.deepEqual(loadSaved(bank)?.answers.s, ['C']);
  // An administrator who already has a saved exam keeps theirs
  mem.set(LEGACY, JSON.stringify({ ids: ['m'] }));
  adoptLegacySession(7, true);
  assert.equal(mem.has(LEGACY), false);
  assert.deepEqual(loadSaved(bank)?.qs.map((x) => x.id), ['s']);
});

// ---------------------------------------------------------------- practice (v0.6.5)

test('a practice keeps its kind and its checked questions; a checked question without an answer is open again', () => {
  saveSession({
    qs: bank.questions, answers: { s: ['B'], m: ['A'] }, flags: new Set(), idx: 2, elapsedSeconds: 30, mode: 'all', sheets: ['S'],
    kind: 'practice', checked: new Set(['s', 'm']),
  });
  let sv = loadSaved(bank);
  assert.equal(sv?.kind, 'practice');
  assert.deepEqual([...(sv?.checked ?? [])].sort(), ['m', 's']);
  // edited by hand: a checked id whose answer is gone, an unknown id and a non-string are dropped
  const raw = JSON.parse(mem.get(KEY) as string) as Record<string, unknown>;
  put({ ...raw, answers: { s: ['B'] }, checked: ['s', 'm', 'nope', 5] });
  sv = loadSaved(bank);
  assert.deepEqual([...(sv?.checked ?? [])], ['s']);
});

test('an exam saves no checked questions, and a save without kind (before v0.6.5) is an exam', () => {
  saveSession({ qs: bank.questions, answers: { s: ['B'] }, flags: new Set(), idx: 0, elapsedSeconds: 0, mode: 'all', sheets: [], kind: 'exam', checked: new Set(['s']) });
  let sv = loadSaved(bank);
  assert.equal(sv?.kind, 'exam');
  assert.equal(sv?.checked.size, 0);
  put({ v: 1, ids: ['s'], answers: { s: ['B'] }, checked: ['s'] });
  sv = loadSaved(bank);
  assert.equal(sv?.kind, 'exam');
  assert.equal(sv?.checked.size, 0);
  put({ v: 1, ids: ['s'], answers: {}, kind: 'weird' });
  assert.equal(loadSaved(bank)?.kind, 'exam');
});
