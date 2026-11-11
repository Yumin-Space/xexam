// What the bank list can tell about a question bank from its summary (GET /api/banks), as pure rules (bank-status.test.ts).
import type { BankSummary } from '../types/api.ts';

/**
 * A bank whose questions are all retired (v0.7). An import always leaves at least one active question (a file without a
 * usable question cannot be imported), so a bank like this held questions from an older Excel layout only: since v0.7 the
 * backend retires those at every start (the bank and its study records stay), and only an update with a six-column file
 * brings the bank back into use. A bank that mixes old-layout and six-column sheets keeps its six-column questions and
 * cannot be told apart from a bank with ordinary retired questions: this says nothing about it.
 */
export function retiredOnly(b: Pick<BankSummary, 'total' | 'inactive'>): boolean {
  return b.total === 0 && b.inactive > 0;
}
