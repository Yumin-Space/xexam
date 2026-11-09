import type { ImportConfirmed, ImportPreview, ImportRecord } from '../types/api.ts';
import { getJson, postFile, postJson } from './client.ts';

export const uploadWorkbook = (file: File): Promise<ImportPreview> => postFile<ImportPreview>('/api/upload', file);
export const confirmImport = (token: string): Promise<ImportConfirmed> => postJson<ImportConfirmed>('/api/import/confirm', { token });
export const cancelImport = (): Promise<{ ok: boolean }> => postJson<{ ok: boolean }>('/api/import/cancel');
export const fetchImports = async (): Promise<ImportRecord[]> => (await getJson<{ imports: ImportRecord[] }>('/api/imports')).imports;
export const shutdownApp = (): Promise<{ ok: boolean }> => postJson<{ ok: boolean }>('/api/shutdown');
