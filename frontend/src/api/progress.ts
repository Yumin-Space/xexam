import type { AttemptRequest, AttemptResponse, Progress, RecordDetail, RecordsDeleted, RecordsResponse } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchProgress = (): Promise<Progress> => getJson<Progress>('/api/progress');
/** Saves an exam; the answer is the new progress plus the ids of the results that were not counted (see AttemptResponse) */
export const submitAttempt = (a: AttemptRequest): Promise<AttemptResponse> => postJson<AttemptResponse>('/api/attempt', a);
/** Clears everything: all exam records and the per-question stats (wrong-answer notebook) */
export const resetProgress = (): Promise<Progress> => postJson<Progress>('/api/reset');

export const fetchRecords = (): Promise<RecordsResponse> => getJson<RecordsResponse>('/api/records');
export const fetchRecord = (id: number): Promise<RecordDetail> => getJson<RecordDetail>(`/api/record?id=${encodeURIComponent(String(id))}`);
/** Delete one exam record (the per-question stats are kept) */
export const deleteRecord = (id: number): Promise<RecordsDeleted> => postJson<RecordsDeleted>('/api/records/delete', { id });
/** Delete every exam record; withStats also empties the per-question stats (wrong-answer notebook) */
export const deleteAllRecords = (withStats: boolean): Promise<RecordsDeleted> =>
  postJson<RecordsDeleted>('/api/records/delete', { all: true, withStats });
