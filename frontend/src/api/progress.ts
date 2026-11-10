import type { AttemptRequest, Progress } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchProgress = (): Promise<Progress> => getJson<Progress>('/api/progress');
export const submitAttempt = (a: AttemptRequest): Promise<Progress> => postJson<Progress>('/api/attempt', a);
export const resetProgress = (): Promise<Progress> => postJson<Progress>('/api/reset');
