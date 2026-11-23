// v2.7 quiz drawer: leaving the quiz by a hash change resets it; a closed drawer carries no dialog semantics. DOM stubs stand in.
import assert from 'node:assert/strict';
import { test } from 'node:test';

type Fn = (e?: unknown) => void;
const g = globalThis as unknown as Record<string, unknown>;

function setup() {
  const listeners = new Map<string, Fn[]>();
  const add = (type: string, fn: Fn): void => { listeners.set(type, [...(listeners.get(type) ?? []), fn]); };
  const classes = new Set<string>();
  const attrs = new Map<string, string>();
  const railClasses = new Set<string>();
  const rail = {
    classList: { toggle: (c: string, on: boolean) => { if (on) railClasses.add(c); else railClasses.delete(c); } },
    setAttribute: (k: string, v: string) => { attrs.set(k, v); },
    removeAttribute: (k: string) => { attrs.delete(k); },
    toggleAttribute: (k: string, on: boolean) => { if (on) attrs.set(k, ''); else attrs.delete(k); },
    focus: () => {},
    querySelector: () => null,
    contains: () => false,
  };
  const root = { classList: { contains: (c: string) => classes.has(c), add: (c: string) => classes.add(c), remove: (c: string) => classes.delete(c) } };
  const loc = { hash: '#/quiz' };
  const hist = { pushed: 0, backs: 0, state: null as unknown };
  let quizInDom = true;
  let observerCb: (() => void) | null = null;
  g.MutationObserver = class { constructor(cb: () => void) { observerCb = cb; } observe() {} };
  g.window = { addEventListener: add, clearTimeout: () => {}, setTimeout: () => 1, scrollY: 0, scrollTo: () => {} };
  g.document = {
    documentElement: root,
    getElementById: (id: string) => (id === 'qz-rail' ? rail : id === 'app' ? {} : null),
    querySelector: (s: string) => (s === '#app .qz' ? (quizInDom ? {} : null) : s === '#qz-rail' ? rail : null),
    addEventListener: add,
    activeElement: null,
  };
  g.location = loc;
  g.history = { pushState: () => { hist.pushed++; }, back: () => { hist.backs++; }, get state() { return hist.state; } };
  g.matchMedia = () => ({ matches: true, addEventListener: () => {} });
  return { listeners, classes, attrs, railClasses, loc, hist, setQuiz: (v: boolean) => { quizInDom = v; }, fireObserver: () => observerCb?.() };
}

test('a hash change away from the quiz resets an open drawer', async () => {
  const env = setup();
  const m = await import('./quiz-drawer.ts');
  m.installSheetListeners();
  m.openSheet();
  assert.equal(m.sheetOpen(), true);
  assert.ok(env.classes.has('qz-sheet-open'));
  assert.equal(env.attrs.get('role'), 'dialog');
  // the route has not been redrawn yet: the quiz is still in the DOM, only the hash changed
  env.loc.hash = '#/study';
  for (const fn of env.listeners.get('hashchange') ?? []) fn();
  assert.equal(m.sheetOpen(), false);
  assert.ok(!env.classes.has('qz-sheet-open'));
  // Back from the new page lands on the stale drawer entry: it is stepped over once
  env.loc.hash = '#/quiz';
  env.hist.state = { qzSheet: true };
  const backs = env.hist.backs;
  for (const fn of env.listeners.get('popstate') ?? []) fn();
  assert.equal(env.hist.backs, backs + 1);
});

test('a page swap without a hash change (sign-in) resets an open drawer', async () => {
  const env = setup();
  const m = await import('./quiz-drawer.ts');
  m.resetSheet();
  m.installSheetListeners();
  m.openSheet();
  assert.ok(env.classes.has('qz-sheet-open'));
  env.setQuiz(false);
  env.fireObserver();
  assert.equal(m.sheetOpen(), false);
  assert.ok(!env.classes.has('qz-sheet-open'));
});

test('a closed drawer is inert and has no dialog role', async () => {
  const env = setup();
  const m = await import('./quiz-drawer.ts');
  m.resetSheet();
  m.applySheet();
  assert.equal(env.attrs.has('role'), false);
  assert.equal(env.attrs.has('aria-modal'), false);
  assert.equal(env.attrs.has('inert'), true);
  m.openSheet();
  assert.equal(env.attrs.get('role'), 'dialog');
  assert.equal(env.attrs.has('inert'), false);
  m.resetSheet();
  m.applySheet();
  assert.equal(env.attrs.has('role'), false);
  assert.equal(env.attrs.has('inert'), true);
});
