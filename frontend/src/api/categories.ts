// v3.1: bank categories (administrators only; the server enforces it). Every answer is the fresh bank list.
import type { BanksResponse, CategoryEdit } from '../types/api.ts';
import { postJson } from './client.ts';

export const editCategories = (e: CategoryEdit): Promise<BanksResponse> => postJson<BanksResponse>('/api/banks/categories', e);
/** Move banks into a category (null = uncategorised), appended at its end in the given order */
export const moveBanks = (ids: number[], categoryId: number | null): Promise<BanksResponse> =>
  postJson<BanksResponse>('/api/banks/category', { ids, categoryId });
