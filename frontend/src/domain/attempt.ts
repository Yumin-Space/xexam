// An exam's results after saving (v0.6.5): the backend counts only questions that still exist and names the others in
// "notCounted" (their bank was deleted while the exam was open). The result page then scores only the counted ones, the same
// numbers as the saved record, and says how many were left out. Pure.
import type { AttemptKind } from '../types/api.ts';

/**
 * Exam or practice (v0.6.5) of a record, a history entry or a saved session. Anything else, or nothing (data from before
 * v0.6.5), is an exam: practice did not exist then.
 */
export function kindOf(x: { kind?: unknown } | null | undefined): AttemptKind {
  return x?.kind === 'practice' ? 'practice' : 'exam';
}

/** `results` without the ones whose question id is in `notCounted`, and how many were left out. */
export function countedResults<T extends { q: { id: string } }>(
  results: readonly T[],
  notCounted: readonly string[] | undefined,
): { counted: T[]; left: number } {
  if (!notCounted?.length) return { counted: results.slice(), left: 0 };
  const gone = new Set(notCounted);
  const counted = results.filter((r) => !gone.has(r.q.id));
  return { counted, left: results.length - counted.length };
}
