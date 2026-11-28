// Debounce and bookkeeping of the saved-session marker reports (v3.1, D.4): no network, the calls and the clock are replaced.
import assert from 'node:assert/strict';
import { beforeEach, test } from 'node:test';
import type { SavedReport } from '../types/api.ts';
import { flushSaved, noteDiscarded, noteFinished, noteSaved, pauseSaved, resumeSaved, setSavedIo } from './saved-marker.ts';
import { setStorageOwner } from './storage-keys.ts';
import { storedSessionKey } from './session-storage.ts';

const mem = new Map<string, string>();
(globalThis as unknown as { localStorage: Storage }).localStorage = {
  getItem: (k: string) => (mem.has(k) ? (mem.get(k) as string) : null),
  setItem: (k: string, v: string) => void mem.set(k, String(v)),
  removeItem: (k: string) => void mem.delete(k),
  clear: () => mem.clear(),
  key: () => null,
  length: 0,
} as Storage;

const K1 = '0123456789abcdef0123456789abcdef';
const K2 = 'fedcba9876543210fedcba9876543210';
let clock = 1000;
const sent: Array<{ r: SavedReport; keepalive: boolean }> = [];
const discarded: string[] = [];

beforeEach(() => {
  mem.clear();
  clock = 1000;
  sent.length = 0;
  discarded.length = 0;
  setStorageOwner(7);
  setSavedIo({
    now: () => clock,
    report: async (r, keepalive = false) => void sent.push({ r, keepalive }),
    discard: async (k) => void discarded.push(k),
  });
  resumeSaved();
});

const rep = (o: Partial<SavedReport> = {}): SavedReport => ({ key: K1, kind: 'practice', answered: 1, total: 10, startedAt: 500, timeLeft: null, ...o });

test('the first save of a session is sent at once, without keepalive', () => {
  noteSaved(rep());
  assert.equal(sent.length, 1);
  assert.equal(sent[0]!.keepalive, false);
});

test('answers within a minute are held; after a minute the next change goes out', () => {
  noteSaved(rep());
  clock += 10;
  noteSaved(rep({ answered: 2 }));
  assert.equal(sent.length, 1);
  clock += 55;
  noteSaved(rep({ answered: 3 }));
  assert.equal(sent.length, 2);
  assert.equal(sent[1]!.r.answered, 3);
  // nothing changed: nothing is sent, however old the last report is
  clock += 500;
  noteSaved(rep({ answered: 3 }));
  assert.equal(sent.length, 2);
});

test('a different session key is sent at once', () => {
  noteSaved(rep());
  clock += 1;
  noteSaved(rep({ key: K2 }));
  assert.equal(sent.length, 2);
  assert.equal(sent[1]!.r.key, K2);
});

test('time running out is sent at once', () => {
  noteSaved(rep({ kind: 'exam', timeLeft: 600 }));
  clock += 1;
  noteSaved(rep({ kind: 'exam', timeLeft: 590 }));
  assert.equal(sent.length, 1);
  clock += 1;
  noteSaved(rep({ kind: 'exam', timeLeft: 0 }));
  assert.equal(sent.length, 2);
  assert.equal(sent[1]!.r.timeLeft, 0);
});

test('flush sends the pending save with keepalive, and only when something is pending and different', () => {
  flushSaved();
  assert.equal(sent.length, 0);
  noteSaved(rep());
  flushSaved();
  assert.equal(sent.length, 1, 'already sent');
  clock += 5;
  noteSaved(rep({ answered: 4 }));
  flushSaved();
  assert.equal(sent.length, 2);
  assert.equal(sent[1]!.keepalive, true);
  assert.equal(sent[1]!.r.answered, 4);
  flushSaved();
  assert.equal(sent.length, 2, 'nothing pending any more');
});

test('flush also carries a changed time left', () => {
  noteSaved(rep({ kind: 'exam', timeLeft: 900 }));
  clock += 20;
  noteSaved(rep({ kind: 'exam', timeLeft: 880 }));
  flushSaved();
  assert.equal(sent.length, 2);
  assert.equal(sent[1]!.r.timeLeft, 880);
});

test('discard tells the server and forgets the pending save', () => {
  noteSaved(rep());
  clock += 5;
  noteSaved(rep({ answered: 2 }));
  noteDiscarded(K1);
  assert.deepEqual(discarded, [K1]);
  flushSaved();
  assert.equal(sent.length, 1, 'nothing resurrected');
  // the same key is a new session to the server again
  noteSaved(rep());
  assert.equal(sent.length, 2);
});

test('finished: forgets locally, no call', () => {
  noteSaved(rep());
  clock += 5;
  noteSaved(rep({ answered: 2 }));
  noteFinished(K1);
  flushSaved();
  assert.equal(sent.length, 1);
  assert.deepEqual(discarded, []);
});

test('nothing is reported with nobody signed in or while paused', () => {
  setStorageOwner(null);
  noteSaved(rep());
  noteDiscarded(K1);
  flushSaved();
  assert.equal(sent.length + discarded.length, 0);
  setStorageOwner(7);
  pauseSaved();
  noteSaved(rep());
  flushSaved();
  assert.equal(sent.length, 0);
  resumeSaved();
  noteSaved(rep());
  assert.equal(sent.length, 1);
});

test('storedSessionKey: the raw key, independent of the question bank', () => {
  assert.equal(storedSessionKey(), null);
  mem.set('xexam.u7.session.v1', JSON.stringify({ ids: ['gone'], key: K1, startedAt: 5 }));
  assert.equal(storedSessionKey(), K1);
  mem.set('xexam.u7.session.v1', JSON.stringify({ ids: ['x'], key: 'NOT-HEX' }));
  assert.equal(storedSessionKey(), null);
  mem.set('xexam.u7.session.v1', JSON.stringify({ ids: ['x'] }));
  assert.equal(storedSessionKey(), null);
  mem.set('xexam.u7.session.v1', 'not json');
  assert.equal(storedSessionKey(), null);
  setStorageOwner(null);
  mem.set('xexam.u7.session.v1', JSON.stringify({ key: K1 }));
  assert.equal(storedSessionKey(), null);
});

test('a report that goes out while the page is hidden uses keepalive', () => {
  noteSaved(rep());
  clock += 70;
  const g = globalThis as { document?: unknown };
  const before = g.document;
  g.document = { visibilityState: 'hidden' };
  try {
    noteSaved(rep({ answered: 5 }));
  } finally {
    g.document = before;
  }
  assert.equal(sent.length, 2);
  assert.equal(sent[1]!.keepalive, true);
});
