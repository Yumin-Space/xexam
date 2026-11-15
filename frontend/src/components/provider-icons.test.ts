// v1.6 provider marks: one aria-hidden svg per family, monochrome, at least one path. A tiny document stub stands in for the DOM.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { PROVIDER_NAME, providerIcon } from './provider-icons.ts';

interface El { tag: string; attrs: Record<string, string>; kids: El[]; setAttribute(k: string, v: string): void; appendChild(c: El): void }
const mk = (_ns: string, tag: string): El => {
  const e: El = { tag, attrs: {}, kids: [], setAttribute(k, v) { e.attrs[k] = v; }, appendChild(c) { e.kids.push(c); } };
  return e;
};

test('each family gives a monochrome hidden svg with a path', () => {
  (globalThis as unknown as { document: unknown }).document = { createElementNS: mk };
  for (const f of ['anthropic', 'openai', 'google'] as const) {
    const s = providerIcon(f) as unknown as El;
    assert.equal(s.tag, 'svg');
    assert.equal(s.attrs['aria-hidden'], 'true');
    assert.equal(s.attrs.fill, 'currentColor');
    assert.ok(s.kids.filter((k) => k.tag === 'path').length >= 1);
    assert.ok((s.kids[0]?.attrs.d ?? '').length > 10);
  }
  delete (globalThis as unknown as { document?: unknown }).document;
});

test('provider names', () => {
  assert.deepEqual(PROVIDER_NAME, { anthropic: 'Anthropic', openai: 'OpenAI', google: 'Google' });
});
