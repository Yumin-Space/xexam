// v2.5: every dashboard visit reads the records again beside the dashboard data, and the newest-record time the client sends
// for the AI study assessment is the largest `at` (the server's MAX(at)), never the first row by id.
// Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { loadVisit } from './dash-visit.ts';

const src = (rel: string): string => readFileSync(new URL(rel, import.meta.url), 'utf8');

test('loadVisit asks for the records on every call, beside the dashboard', async () => {
  let dash = 0;
  let recs = 0;
  const f = { dashboard: async () => ++dash, records: async () => ++recs };
  assert.deepEqual(await loadVisit(f), { data: 1, records: 1 });
  assert.deepEqual(await loadVisit(f), { data: 2, records: 2 });
});

test('loadVisit: a failed records request does not fail the dashboard, and the other way round', async () => {
  const boom = async (): Promise<never> => { throw new Error('x'); };
  assert.deepEqual(await loadVisit({ dashboard: async () => 'd', records: boom }), { data: 'd', records: null });
  assert.deepEqual(await loadVisit({ dashboard: boom, records: async () => 'r' }), { data: null, records: 'r' });
});

test('the dashboard visit refreshes S.records through loadVisit', () => {
  const s = src('./dashboard.ts');
  assert.match(s, /loadVisit\(\{\s*dashboard:\s*fetchDashboard,\s*records:\s*fetchRecords\s*\}\)/);
  assert.match(s, /S\.records = records\.records;/);
});

test('the assessment button and the greeting take the newest time from newestRecordAt, not records[0]', () => {
  const ctl = src('../components/ai-plus-controls.ts');
  const body = ctl.slice(ctl.indexOf('export function assessButton'), ctl.indexOf('export function aiTableNote'));
  assert.match(body, /newestRecordAt\(records\)/);
  assert.doesNotMatch(body, /records\[0\]/);
  const page = src('../pages/dashboard.ts');
  const hello = page.slice(page.indexOf('function hello'), page.indexOf('function aiRow'));
  assert.match(hello, /newestRecordAt\(S\.records\)/);
  assert.doesNotMatch(hello, /rec\[0\]|records\[0\]/);
});
