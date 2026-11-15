import type { Bank, BankDeleted, BanksResponse, BankSummary, Info } from '../types/api.ts';
import { downloadPath, type DownloadFormat } from '../domain/bank-download.ts';
import { getFile, getJson, postJson, type FileDownload } from './client.ts';

export const fetchInfo = (): Promise<Info> => getJson<Info>('/api/info');
/** All active questions of all banks (each question carries its `bank` id) */
export const fetchBank = (): Promise<Bank> => getJson<Bank>('/api/bank');
export const fetchBanks = async (): Promise<BankSummary[]> => (await getJson<BanksResponse>('/api/banks')).banks;
/** Delete a bank. withRecords also deletes its study data (stats, per-question detail, records that only contain this bank). */
export const deleteBank = (id: number, withRecords: boolean): Promise<BankDeleted> =>
  postJson<BankDeleted>('/api/banks/delete', { id, withRecords });

/**
 * v1.1 (C-B1): delete some sheets of a bank (never all of them, never an AI bank's). The answer is the same as the bank delete's.
 * `withRecords` also deletes the study data of the deleted questions.
 */
export const deleteSheets = (id: number, sheets: readonly string[], withRecords: boolean): Promise<BankDeleted> =>
  postJson<BankDeleted>('/api/banks/sheets/delete', { id, sheets, withRecords });

/** v1.6: one bank as .xlsx (import format) or .docx (for reading). Default time limit 120 s. */
export const downloadBank = (id: number, format: DownloadFormat): Promise<FileDownload> =>
  getFile(downloadPath(id, format), { timeoutMs: 120_000 });
