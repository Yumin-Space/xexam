import type { Bank, Info } from '../types/api.ts';
import { getJson } from './client.ts';

export const fetchInfo = (): Promise<Info> => getJson<Info>('/api/info');
export const fetchBank = (): Promise<Bank> => getJson<Bank>('/api/bank');
