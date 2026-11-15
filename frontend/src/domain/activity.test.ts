import assert from 'node:assert/strict';
import { test } from 'node:test';
import { actionFilter, deviceLabel, detailText, hardSoonMinutes, looksLikeProxy, mergeNewer, userFilter } from './activity.ts';

test('deviceLabel: browser and system, unknown agents cut, none = null', () => {
  assert.equal(deviceLabel('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36'), 'Chrome · Windows');
  assert.equal(deviceLabel('Mozilla/5.0 (Windows NT 10.0) AppleWebKit/537.36 Chrome/120.0 Safari/537.36 Edg/120.0'), 'Edge · Windows');
  assert.equal(deviceLabel('Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605 Version/17.0 Mobile Safari/604.1'), 'Safari · iOS');
  assert.equal(deviceLabel('Mozilla/5.0 (X11; Linux x86_64; rv:120.0) Gecko/20100101 Firefox/120.0'), 'Firefox · Linux');
  assert.equal(deviceLabel('x'.repeat(100)), 'x'.repeat(40));
  assert.equal(deviceLabel(''), null);
  assert.equal(deviceLabel(null), null);
});

test('detailText: key reuse reads old -> new, others key: value, long text cut', () => {
  assert.equal(detailText({ oldHost: 'old.example.com', newHost: 'api.example.com' }), 'old.example.com → api.example.com');
  assert.equal(detailText({ count: 2, keys: ['a', 'b'] }), 'count: 2; keys: a, b');
  assert.equal(detailText(null), '');
  assert.ok(detailText({ host: 'h'.repeat(400) }).length <= 160);
});

test('filters: user and action values', () => {
  assert.equal(userFilter(''), undefined);
  assert.equal(userFilter('system'), 'system');
  assert.equal(userFilter('anon'), 'anon');
  assert.equal(userFilter('7'), 7);
  assert.equal(userFilter('-1'), undefined);
  assert.deepEqual(actionFilter('g:auth'), { group: 'auth' });
  assert.deepEqual(actionFilter('a:page.view'), { action: 'page.view' });
  assert.deepEqual(actionFilter(''), {});
});

test('hardSoonMinutes: only inside the last ten minutes', () => {
  const end = Date.UTC(2026, 10, 21, 7, 20, 0);
  const at = '2026-11-21 07:20:00';
  assert.equal(hardSoonMinutes(at, end - 11 * 60_000), null);
  assert.equal(hardSoonMinutes(at, end - 9 * 60_000 - 1), 10);
  assert.equal(hardSoonMinutes(at, end - 30_000), 1);
  assert.equal(hardSoonMinutes(at, end), null);
  assert.equal(hardSoonMinutes(null, end), null);
});

test('mergeNewer: no duplicates, newest first', () => {
  assert.deepEqual(mergeNewer([{ id: 3 }, { id: 2 }], [{ id: 5 }, { id: 3 }]).map((e) => e.id), [5, 3, 2]);
});

test('looksLikeProxy: one private address on three or more entries', () => {
  assert.equal(looksLikeProxy(['172.18.0.2', '172.18.0.2', '172.18.0.2', null]), true);
  assert.equal(looksLikeProxy(['172.18.0.2', '172.18.0.2']), false);
  assert.equal(looksLikeProxy(['203.0.113.7', '203.0.113.7', '203.0.113.7']), false);
  assert.equal(looksLikeProxy(['10.0.0.1', '10.0.0.2', '10.0.0.1']), false);
});
