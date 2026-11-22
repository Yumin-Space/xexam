// The page-visit report (v1.3, activity log): which route change is reported. Pure; state/actions.ts sends it.
import type { VisitPage } from '../types/api.ts';

const PAGES: readonly string[] = ['home', 'study', 'banks', 'users', 'logs', 'records', 'record', 'wrong', 'settings', 'profile', 'about', 'quiz', 'result', 'ai', 'aiSettings'];

/**
 * The page to report for a move from route `from` (null = the page was just opened) to route `to`, or null for nothing: only
 * while the server asked for visits (`logVisits`), only when the route NAME changes (another record, another chat or a
 * redraw is not a new visit), and only for a name the server knows.
 */
export function visitToReport(logVisits: boolean, from: string | null, to: string): VisitPage | null {
  if (!logVisits || from === to || !PAGES.includes(to)) return null;
  return to as VisitPage;
}
