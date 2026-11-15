// The external IP-location lookup settings (v1.4, administrators): the routes live in ai-service (`/api/ai/admin/geo*`), which
// holds the provider settings and the sealed key and makes the outbound call. The key is never returned, only whether one is set.
import type { GeoDraft, GeoSaveAnswer, GeoSettings, GeoSource, GeoState, GeoTestAnswer } from '../types/api.ts';
import { getJson, postJson } from './client.ts';

export const fetchGeo = (): Promise<GeoState> => getJson<GeoState>('/api/ai/admin/geo');
/** Looks up the fixed public example IP through the draft; saves nothing */
export const testGeo = (draft: GeoDraft): Promise<GeoTestAnswer> => postJson<GeoTestAnswer>('/api/ai/admin/geo/test', draft);
/** Test and save: a failed test stores nothing (`ok: false`) */
export const saveGeo = (draft: GeoDraft): Promise<GeoSaveAnswer> => postJson<GeoSaveAnswer>('/api/ai/admin/geo/save', draft);
/** Switches the place source to the external API (409 `ai.geo.testFirst` without saved settings) */
export const enableGeo = (): Promise<{ source: GeoSource }> => postJson<{ source: GeoSource }>('/api/ai/admin/geo/enable', {});
/** Deletes the settings and the key; the source goes back to Cloudflare */
export const deleteGeo = (): Promise<{ settings: GeoSettings | null; source: GeoSource }> =>
  postJson<{ settings: GeoSettings | null; source: GeoSource }>('/api/ai/admin/geo/delete', {});
