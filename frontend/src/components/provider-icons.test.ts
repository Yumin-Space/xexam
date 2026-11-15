// v1.6 provider marks: one aria-hidden svg per family, monochrome, at least one path. A tiny document stub stands in for the DOM.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { EFFORT_FAMILIES, effortAria, effortTip, PROVIDER_NAME, providerIcon } from './provider-icons.ts';

interface El { tag: string; attrs: Record<string, string>; kids: El[]; setAttribute(k: string, v: string): void; appendChild(c: El): void }
const mk = (_ns: string, tag: string): El => {
  const e: El = { tag, attrs: {}, kids: [], setAttribute(k, v) { e.attrs[k] = v; }, appendChild(c) { e.kids.push(c); } };
  return e;
};

test('each family gives a monochrome hidden svg with a path', () => {
  (globalThis as unknown as { document: unknown }).document = { createElementNS: mk };
  for (const f of ['anthropic', 'openai', 'google', 'responses', 'gemma'] as const) {
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
  assert.deepEqual(PROVIDER_NAME, { anthropic: 'Anthropic', openai: 'OpenAI', google: 'Google', responses: 'Responses API', gemma: 'Gemma' });
  assert.deepEqual(Object.keys(PROVIDER_NAME), ['anthropic', 'openai', 'google', 'responses', 'gemma']);
  assert.deepEqual(EFFORT_FAMILIES, ['anthropic', 'openai', 'google', 'responses', 'gemma'], 'row order after [Default]');
});

test('tooltip is the parameter sent, aria-label carries provider and parameter', () => {
  assert.deepEqual(EFFORT_FAMILIES.map(effortTip), ['output_config.effort', 'reasoning_effort', 'thinking_level', 'reasoning.effort', 'thinking_level']);
  assert.equal(effortAria('anthropic'), 'Anthropic, output_config.effort');
  assert.equal(effortAria('responses'), 'Responses API, reasoning.effort');
  assert.equal(effortAria('gemma'), 'Gemma, thinking_level');
});
