// The administrators' AI settings routes (v1.3, `/api/ai/admin/*`): connections, the model pool and slots, the global
// feature switches, limits, usage, clearing data and the read-only chat viewer. Every write answers with the whole
// settings object (`settings`). The tests run long (the server's whole-request deadline is 90 s), so those calls wait a
// little longer than that.
import type {
  AiChatDetail, AiClear, AiConnectionSave, AiFeatureKey, AiLimits, AiLimitsAnswer, AiModelList, AiSettings, AiSlot,
  AiTestAnswer, AiUsage, ModelRef,
} from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchAiSettings = (): Promise<AiSettings> => getJson<AiSettings>('/api/ai/admin/settings');

/** `POST /api/ai/admin/settings`: only the retention days remain here */
export const saveRetention = (retentionDays: number): Promise<AiSettings> =>
  postJson<AiSettings>('/api/ai/admin/settings', { retentionDays });

/** Test before save: a pass saves (`saved: true`), a failure saves nothing; key reuse needs `confirmHost` (409 `ai.keyReuseConfirm`) */
export const saveConnection = (c: AiConnectionSave): Promise<AiTestAnswer> => postJson<AiTestAnswer>('/api/ai/admin/connections/save', c);
export const testConnection = (id: number): Promise<AiTestAnswer> => postJson<AiTestAnswer>('/api/ai/admin/connections/test', { id });
/** `force`: the features that use the connection are emptied (409 `ai.connectionInUse` otherwise); `newDefault` when it held the default */
export const deleteConnection = (id: number, opts: { force?: boolean; newDefault?: ModelRef } = {}): Promise<{ settings: AiSettings }> =>
  postJson<{ settings: AiSettings }>('/api/ai/admin/connections/delete', { id, force: opts.force === true, ...(opts.newDefault ? { newDefault: opts.newDefault } : {}) });
export const orderConnections = (ids: number[]): Promise<{ settings: AiSettings }> =>
  postJson<{ settings: AiSettings }>('/api/ai/admin/connections/order', { ids });

/** The models of every connection (cached 10 min; `refresh` lists again); also read by the AI Chat model picker */
export const fetchModels = (refresh = false): Promise<AiModelList> => getJson<AiModelList>(`/api/ai/admin/models?refresh=${refresh ? 1 : 0}`);

export type PickBody = { slot: AiSlot; connectionId: number; model: string; manual: boolean } | { slot: Exclude<AiSlot, 'default'>; clear: true };
export interface PickAnswer {
  ok: boolean;
  code: string | null;
  httpStatus: number | null;
  latencyMs: number | null;
  settings: AiSettings;
}
export const pickModel = (b: PickBody): Promise<PickAnswer> => postJson<PickAnswer>('/api/ai/admin/models/pick', b);
export const testModel = (connectionId: number, model: string): Promise<AiTestAnswer> =>
  postJson<AiTestAnswer>('/api/ai/admin/models/test', { connectionId, model });
export const removeModel = (poolId: number): Promise<{ settings: AiSettings }> =>
  postJson<{ settings: AiSettings }>('/api/ai/admin/models/remove', { poolId });

/** `default` is required while the default slot is empty; a failing test answers `ok: false` and leaves AI+ off */
export const setEnabled = (enabled: boolean, def?: ModelRef): Promise<PickAnswer> =>
  postJson<PickAnswer>('/api/ai/admin/enable', def ? { enabled, default: def } : { enabled });
/** Any subset of the six switches; they apply to normal users only (administrators are not affected) */
export const setFeatureSwitches = (s: Partial<Record<AiFeatureKey, boolean>>): Promise<AiSettings> =>
  postJson<AiSettings>('/api/ai/admin/feature-switches', s);

export const fetchLimits = (): Promise<AiLimitsAnswer> => getJson<AiLimitsAnswer>('/api/ai/admin/limits');
export const saveLimits = (l: Partial<AiLimits>): Promise<AiLimitsAnswer> => postJson<AiLimitsAnswer>('/api/ai/admin/limits', l);
export const fetchUsage = (days: number): Promise<AiUsage> => getJson<AiUsage>(`/api/ai/admin/usage?days=${days}`);

export type AiClearV13 = AiClear | { what: 'shared' };
export const clearAiData = (c: AiClearV13): Promise<{ deleted: number }> => postJson<{ deleted: number }>('/api/ai/admin/clear', c);

/** A user's current branch; `at` = a read-only view of the branch that ends at that message (never changes what the user sees) */
export const adminChat = (user: number, id: number, at?: number): Promise<AiChatDetail> =>
  getJson<AiChatDetail>(`/api/ai/admin/chat?user=${user}&id=${id}${at !== undefined ? `&at=${at}` : ''}`);
