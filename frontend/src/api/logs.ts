// System log (v0.7, administrators only): the recent log lines the backend keeps in memory.
import { chunkOf, type LogChunk } from '../domain/logs.ts';
import { getJson } from './client.ts';

/**
 * GET /api/logs: the lines after `after` (all kept lines when null), checked into the page's form. `boot` is the backend
 * process the page last saw: when the backend has restarted since, it answers with every line it keeps.
 */
export const fetchLogs = async (after: number | null, boot: string | null = null): Promise<LogChunk> => {
  const q = new URLSearchParams();
  if (after != null) q.set('after', String(after));
  if (after != null && boot != null) q.set('boot', boot);
  const qs = q.toString();
  return chunkOf(await getJson<unknown>(qs ? `/api/logs?${qs}` : '/api/logs'));
};
