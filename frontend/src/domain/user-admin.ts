// User management's AI+ rules (v0.8 round 2; contracts §2.8), without DOM so they run under `node --test`.
// The per-user "AI+ service" switch is shown, and can be used, only while AI+ is available: ai-service is running and
// connected to the main service (a report younger than 60 s) and an administrator turned the AI+ service on in AI
// settings. `aiAvailable` comes with every answer of GET /api/users and POST /api/users/*. Otherwise the rows have no AI+
// line and the New user form no AI+ box; the stored switches stay as they are. A page drawn before AI+ went away that
// still tries is refused by the backend (409, code `ai.serviceOff`): no success message then, only the refusal.
import type { Role } from '../types/api.ts';

/** The code (and the backend's Chinese text) of the refusal while AI+ is not available */
export const AI_SERVICE_OFF_CODE = 'ai.serviceOff';
export const MSG_AI_SERVICE_OFF = '请先在 AI 设置里开启 AI+ 功能。';

/** The AI+ line of a row: none while AI+ is not available; a switch for a plain user; a note for an administrator */
export type AiLineKind = 'none' | 'switch' | 'admin';

export function aiLineKind(aiAvailable: boolean, role: Role): AiLineKind {
  if (!aiAvailable) return 'none';
  return role === 'admin' ? 'admin' : 'switch';
}

/** Whether a plain user's AI+ service switch (row) or box (New user form) is drawn */
export const aiSwitchShown = (aiAvailable: boolean, role: Role): boolean => aiLineKind(aiAvailable, role) === 'switch';

/** The `ai` of a create body: true only when the AI+ box is drawn (AI+ available, role user) and ticked */
export const createAi = (aiAvailable: boolean, role: Role, ticked: boolean): boolean => aiSwitchShown(aiAvailable, role) && ticked;

/** The refusal "Turn on the AI+ service in AI settings first" (an ApiError, or anything with that code or text) */
export function isServiceOff(e: unknown): boolean {
  if (typeof e !== 'object' || e === null) return false;
  const x = e as { code?: unknown; serverMessage?: unknown };
  return x.code === AI_SERVICE_OFF_CODE || x.serverMessage === MSG_AI_SERVICE_OFF;
}
