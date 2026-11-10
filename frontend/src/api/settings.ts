import type { Settings } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchSettings = (): Promise<Settings> => getJson<Settings>('/api/settings');
export const saveSettings = (patch: Partial<Settings>): Promise<Settings> => postJson<Settings>('/api/settings', patch);
/** Restores theme, UI language and auto-advance to their defaults */
export const resetSettings = (): Promise<Settings> => postJson<Settings>('/api/settings/reset');
