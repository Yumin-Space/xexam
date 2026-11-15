// Activity log (v1.3, administrators): the paged list, its settings, and the visit report every signed-in page sends.
import type { ActivityPage, ActivitySettings, VisitPage } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export interface ActivityQuery {
  before?: number;
  after?: number;
  limit?: number;
  user?: number | 'system' | 'anon';
  group?: string;
  action?: string;
  result?: 'ok' | 'fail';
  facets?: boolean;
}

/** The query string of `GET /api/activity` (only the keys that are set; `facets` as 1) */
export function activityQuery(q: ActivityQuery): string {
  const p = new URLSearchParams();
  if (q.before !== undefined) p.set('before', String(q.before));
  if (q.after !== undefined) p.set('after', String(q.after));
  if (q.limit !== undefined) p.set('limit', String(q.limit));
  if (q.user !== undefined) p.set('user', String(q.user));
  if (q.group) p.set('group', q.group);
  if (q.action) p.set('action', q.action);
  if (q.result) p.set('result', q.result);
  if (q.facets) p.set('facets', '1');
  const s = p.toString();
  return s ? `?${s}` : '';
}

/** One read may take this long (the page asks again only after a read has settled) */
export const ACTIVITY_READ_TIMEOUT_MS = 10_000;

export const fetchActivity = (q: ActivityQuery = {}): Promise<ActivityPage> =>
  getJson<ActivityPage>(`/api/activity${activityQuery(q)}`, { timeoutMs: ACTIVITY_READ_TIMEOUT_MS });

export const fetchActivitySettings = (): Promise<ActivitySettings> => getJson<ActivitySettings>('/api/activity/settings');

export const saveActivitySettings = (s: { retentionDays?: number; userVisits?: boolean }): Promise<ActivitySettings> =>
  postJson<ActivitySettings>('/api/activity/settings', s);

/** Reports that the signed-in user opened a page; every error is swallowed (it never disturbs the page) */
export async function reportVisit(page: VisitPage): Promise<void> {
  try {
    await postJson<{ ok: boolean }>('/api/activity/visit', { page });
  } catch {
    // a lost report is not worth a message
  }
}
