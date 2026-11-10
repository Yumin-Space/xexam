import type { ImportConfirmed, ImportPreview, ImportRecord, ImportTarget } from '../types/api.ts';
import { getJson, postFile, postJson } from './client.ts';

/** Upload a workbook for preview. `target` = 'new' creates a new bank on confirm; a bank id updates / replaces that bank. */
export const uploadWorkbook = (file: File, target: ImportTarget): Promise<ImportPreview> =>
  postFile<ImportPreview>('/api/upload', file, { 'X-Bank-Target': String(target) });
export const confirmImport = (token: string): Promise<ImportConfirmed> => postJson<ImportConfirmed>('/api/import/confirm', { token });
export const cancelImport = (): Promise<{ ok: boolean }> => postJson<{ ok: boolean }>('/api/import/cancel');
export const fetchImports = async (): Promise<ImportRecord[]> => (await getJson<{ imports: ImportRecord[] }>('/api/imports')).imports;
export const shutdownApp = (): Promise<{ ok: boolean }> => postJson<{ ok: boolean }>('/api/shutdown');
