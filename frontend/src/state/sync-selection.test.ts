// syncSelection (v2.4, owner): nothing is ticked by default - not on the first load, not for a bank that appears later;
// a ticked sheet that vanished is dropped. Made-up banks only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { S, sheetKey, syncSelection } from './store.ts';
import type { BankSummary } from '../types/api.ts';

const bank = (id: number, ai: boolean): BankSummary => ({ id, name: `Demo ${id}`, ai, sheets: [{ name: 'S1', usable: 3 }] }) as unknown as BankSummary;

test('nothing is ticked by default, new banks stay unticked, a vanished one is dropped', () => {
  S.banks = [bank(1, false)];
  S.cfg.selected = null;
  S.cfg.known = [];
  syncSelection();
  assert.deepEqual([...(S.cfg.selected ?? [])], [], 'the first load ticks nothing');
  (S.cfg.selected as Set<string>).add(sheetKey(1, 'S1'));
  S.banks = [bank(1, false), bank(2, true), bank(3, false)];
  syncSelection();
  const sel = S.cfg.selected as Set<string>;
  assert.ok(sel.has(sheetKey(1, 'S1')), 'what the user ticked stays ticked');
  assert.ok(!sel.has(sheetKey(2, 'S1')) && !sel.has(sheetKey(3, 'S1')), 'new banks (AI or not) stay unticked');
  S.banks = [bank(2, true), bank(3, false)];
  syncSelection();
  assert.ok(!(S.cfg.selected as Set<string>).has(sheetKey(1, 'S1')), 'a vanished sheet is dropped');
});
