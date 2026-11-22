// v2.5: the Home dashboard's one read-only request, GET /api/dashboard (leaderboard + most practised questions).
// The types live here so types/api.ts stays untouched. The server sends no user ids, no e-mail domains and no question text.
import { getJson } from './client.ts';

export interface RankEntry {
  rank: number;
  /** Already masked by the server (an e-mail username shows only a masked local part); the viewer's own row is in full */
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

export interface PopularItem {
  questionId: string;
  bankId: number;
  times: number;
  correct: number;
}

export interface Dashboard {
  ranking: { users: number; top: RankEntry[]; me: RankMe };
  popular: { days: number; since: string; items: PopularItem[] };
}

export const fetchDashboard = (): Promise<Dashboard> => getJson<Dashboard>('/api/dashboard');
