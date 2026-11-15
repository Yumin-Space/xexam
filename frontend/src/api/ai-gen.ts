// The AI bank generation routes (v1.1, API-U2 .. U5, API-A1): start, poll one job, cancel, dismiss, and the administrators'
// list. Answers are parsed with the tolerant readers of state/ai.ts (a missing key falls back, never throws).
import { genStatusOf, jobOf } from '../state/ai.ts';
import type { GenAdminJob, GenItem, GenJob, GenJobAnswer, GenStatus } from '../types/api.ts';
import { ApiError, getJson, postJson } from './client.ts';

export type StartBody =
  | { source: 'bank'; bankId: number }
  | { source: 'scope' | 'wrong'; sheets: Array<{ bank: number; sheet: string }>; name: string };

export interface StartAnswer {
  job: GenJob;
  gen: GenStatus | undefined;
}

/** `POST /api/ai/gen/start` (202) */
export async function startGen(body: StartBody): Promise<StartAnswer> {
  const a = await postJson<Record<string, unknown>>('/api/ai/gen/start', body);
  const job = jobOf(a['job']);
  if (!job) throw new ApiError('unexpected', 202);
  return { job, gen: genStatusOf(a['gen']) };
}

/**
 * `GET /api/ai/gen/job?id=[&after=<no>&lang=]`. Without `after`: the job alone (as before). With it (v1.3): the job and the
 * finished items after number `after` in `lang` (owner only; at most 20 per answer; `gone` = numbers dropped by the checks).
 */
export async function fetchGenJob(id: number): Promise<GenJob>;
export async function fetchGenJob(id: number, after: number, lang?: string): Promise<GenJobAnswer>;
export async function fetchGenJob(id: number, after?: number, lang?: string): Promise<GenJob | GenJobAnswer> {
  if (after !== undefined) {
    const q = `id=${id}&after=${after}${lang ? `&lang=${lang}` : ''}`;
    const r = await getJson<Record<string, unknown>>(`/api/ai/gen/job?${q}`, { timeoutMs: 10_000 });
    const j = jobOf(r['job']);
    if (!j) throw new ApiError('unexpected', 200);
    const out: GenJobAnswer = { job: j };
    if (Array.isArray(r['items'])) out.items = r['items'] as GenItem[];
    if (typeof r['last'] === 'number') out.last = r['last'];
    if (Array.isArray(r['gone'])) out.gone = (r['gone'] as unknown[]).filter((n): n is number => typeof n === 'number');
    return out;
  }
  const a = await getJson<Record<string, unknown>>(`/api/ai/gen/job?id=${id}`, { timeoutMs: 10_000 });
  const job = jobOf(a['job']);
  if (!job) throw new ApiError('unexpected', 200);
  return job;
}

/** `POST /api/ai/gen/cancel {id, keep?}` -> the cancelled job; `keep` (A-OWNER-66) saves the finished questions of an import as a bank */
export async function cancelGen(id: number, keep = false): Promise<GenJob> {
  const a = await postJson<Record<string, unknown>>('/api/ai/gen/cancel', keep ? { id, keep: true } : { id });
  const job = jobOf(a['job']);
  if (!job) throw new ApiError('unexpected', 200);
  return job;
}

/** `POST /api/ai/gen/dismiss {id}` (finished jobs; idempotent) */
export const dismissGen = (id: number): Promise<{ ok: boolean }> => postJson<{ ok: boolean }>('/api/ai/gen/dismiss', { id });

/** The Job a 409 `ai.gen.busy` / `ai.import.notWaiting` / `ai.gen.tooLate` carries (null when it has none, SEC-7) */
export function jobOfError(e: unknown): GenJob | null {
  return e instanceof ApiError ? jobOf(e.body?.['job']) : null;
}

/** `GET /api/ai/admin/gen?limit=` (administrators) */
export async function fetchAdminGen(limit = 50): Promise<GenAdminJob[]> {
  const a = await getJson<{ jobs?: unknown }>(`/api/ai/admin/gen?limit=${limit}`);
  const rows = Array.isArray(a.jobs) ? a.jobs : [];
  return rows.flatMap((r): GenAdminJob[] => {
    const j = jobOf(r);
    const o = r as Record<string, unknown>;
    if (!j) return [];
    return [{
      ...j, userId: typeof o['userId'] === 'number' ? o['userId'] : 0, username: typeof o['username'] === 'string' ? o['username'] : '',
      tokensIn: typeof o['tokensIn'] === 'number' ? o['tokensIn'] : 0, tokensOut: typeof o['tokensOut'] === 'number' ? o['tokensOut'] : 0,
    }];
  });
}
