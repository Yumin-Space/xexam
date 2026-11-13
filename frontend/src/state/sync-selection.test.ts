// syncSelection (A-OWNER-64.2): a new AI bank appears unticked, a new ordinary bank is ticked. Made-up banks only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { S, sheetKey, syncSelection } from './store.ts';
import type { BankSummary } from '../types/api.ts';

const bank = (id: number, ai: boolean): BankSummary => ({ id, name: `Demo ${id}`, ai, sheets: [{ name: 'S1', usable: 3 }] }) as unknown as BankSummary;

test('a new AI bank is not ticked, a new ordinary bank is, a vanished one is dropped', () => {
  S.banks = [bank(1, false)];
  S.cfg.selected = null;
  S.cfg.known = [];
  syncSelection();
  assert.deepEqual([...(S.cfg.selected ?? [])], [sheetKey(1, 'S1')]);
  S.banks = [bank(1, false), bank(2, true), bank(3, false)];
  syncSelection();
  const sel = S.cfg.selected as Set<string>;
  assert.ok(sel.has(sheetKey(1, 'S1')) && sel.has(sheetKey(3, 'S1')));
  assert.ok(!sel.has(sheetKey(2, 'S1')), 'the AI bank stays unticked');
  assert.ok(S.cfg.known.includes(sheetKey(2, 'S1')), 'but it is known, so it is never auto-ticked later');
  S.banks = [bank(2, true), bank(3, false)];
  syncSelection();
  assert.ok(!(S.cfg.selected as Set<string>).has(sheetKey(1, 'S1')));
});
