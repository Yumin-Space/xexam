// v1.3: Regenerate sends fresh: true (nothing else does) and the start event's `shared` flag is remembered.
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { buildStudyAsk } from './ai-plus.ts';
import { freshSpec, isSharedSpec, sharedOfStart, studySpec, summarySpec, type StudyTransport } from './ai-spec.ts';

const bodies: Array<Record<string, unknown>> = [];
const tr: StudyTransport = {
  savedStudy: async () => ({ chatId: null, latest: null, lastGood: null }),
  askStudy: async (b) => { bodies.push(b as unknown as Record<string, unknown>); },
  savedSummary: async () => ({ chatId: null, latest: null, lastGood: null }),
  askSummary: async () => {},
  savedAssess: async () => ({ chatId: null, latest: null, lastGood: null }),
  askAssess: async () => {},
  stopChat: async () => {},
};
const ac = new AbortController();
const body = () => buildStudyAsk({
  kind: 'hint', mode: 'practice', key: 'a'.repeat(32), startedAt: 5, questionId: 'q1', no: 1, qlang: 'en', ui: 'en', source: 'practice',
});

describe('v1.3: fresh regenerate and the shared flag', () => {
  it('only the fresh variant sends fresh: true; open / the first ask never does', async () => {
    bodies.length = 0;
    const spec = studySpec(tr, null, body);
    await spec.ask(ac.signal, () => {});
    await freshSpec(spec).ask(ac.signal, () => {});
    assert.equal(bodies[0]!['fresh'], undefined);
    assert.equal(bodies[1]!['fresh'], true);
  });

  it('freshSpec leaves any other spec as it is', () => {
    const s = summarySpec(tr, { id: 3, at: '2026-11-11 10:30' }, () => 'en');
    assert.equal(freshSpec(s), s);
  });

  it('the start event decides the shared flag: true, false', async () => {
    const live: StudyTransport = { ...tr, askStudy: async (_b, _s, on) => { on({ event: 'start', data: { shared: true } }); } };
    const spec = studySpec(live, null, body);
    assert.equal(isSharedSpec(spec), false);
    const seen: string[] = [];
    await spec.ask(ac.signal, (ev) => seen.push(ev.event));
    assert.equal(isSharedSpec(spec), true);
    assert.deepEqual(seen, ['start']);
    const model: StudyTransport = { ...tr, askStudy: async (_b, _s, on) => { on({ event: 'start', data: { shared: false } }); } };
    const spec2 = studySpec(model, null, body);
    await freshSpec(spec2).ask(ac.signal, () => {});
    assert.equal(isSharedSpec(spec2), false);
    assert.equal(spec2.shared, false);
  });

  it('sharedOfStart accepts only a boolean', () => {
    assert.equal(sharedOfStart({ shared: true }), true);
    assert.equal(sharedOfStart({ shared: false }), false);
    assert.equal(sharedOfStart({ shared: 'yes' }), null);
    assert.equal(sharedOfStart(null), null);
    assert.equal(sharedOfStart({}), null);
  });
});
