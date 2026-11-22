// v2.5: pure helpers of the Home dashboard.
import type { PopularItem } from '../api/dashboard.ts';

/** The name the greeting uses: the part of the username before the first "@" (a plain username is kept as it is) */
export function greetName(username: string): string {
  const name = String(username ?? '').trim();
  const at = name.indexOf('@');
  if (at < 0) return name;
  return name.slice(0, at).trim() || name;
}

export interface PopularRow {
  id: string;
  /** The question stem on one line (whitespace collapsed) */
  stem: string;
  bankId: number;
  sheet: string;
  times: number;
}

/** What the "most practised" block needs from the loaded bank */
export interface QuestionSource {
  questions: ReadonlyArray<{ id: string; stem: string; sheet: string; bank: number }>;
}

/** Resolves the server's ids against the loaded bank; an id the client does not know (not loaded, retired) is dropped */
export function popularRows(items: readonly PopularItem[], bank: QuestionSource | null | undefined): PopularRow[] {
  if (!bank) return [];
  const byId = new Map(bank.questions.map((q) => [q.id, q]));
  const rows: PopularRow[] = [];
  for (const it of items) {
    const q = byId.get(it.questionId);
    if (!q) continue;
    rows.push({ id: q.id, stem: q.stem.replace(/\s+/g, ' ').trim(), bankId: it.bankId, sheet: q.sheet, times: it.times });
  }
  return rows;
}

/**
 * The one line the leaderboard shows besides its rows: with nobody ranked only the invitation to answer (an empty install
 * would otherwise say "nobody has answered" and "you have not answered" one under the other); with others ranked the
 * invitation for a viewer who is not; else nothing. `empty` only for a board without rows that still ranks the viewer.
 */
export function rankNote(topCount: number, meRank: number | null): 'noneYou' | 'empty' | null {
  if (meRank === null) return 'noneYou';
  return topCount > 0 ? null : 'empty';
}
