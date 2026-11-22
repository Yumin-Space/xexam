// v2.5: the Home dashboard's one read-only request, GET /api/dashboard (leaderboard + the viewer's most missed questions).
// The types live here so types/api.ts stays untouched. The server sends no user ids, no e-mail domains and no question text; the wrong-answer list is the viewer's own.
import { getJson } from './client.ts';

export interface RankEntry {
  rank: number;
  /** The display name; a user without one is shown masked by the server (an e-mail login keeps only a masked local part) */
  name: string;
  answered: number;
  correct: number;
  self: boolean;
}

/** The viewer's own position; `rank` is null while they have no answers */
export interface RankMe {
  rank: number | null;
  answered: number;
  correct: number;
}

/** One of the viewer's most missed questions: ids and counts only (the text is read from the loaded bank) */
export interface WrongItem {
  questionId: string;
  bankId: number;
  /** How many times the viewer answered it wrong, all time */
  wrong: number;
  attempts: number;
}

export interface Dashboard {
  ranking: { users: number; top: RankEntry[]; me: RankMe };
  /** The viewer's own top 5, most missed first */
  wrongTop: { items: WrongItem[] };
}

export const fetchDashboard = (): Promise<Dashboard> => getJson<Dashboard>('/api/dashboard');
