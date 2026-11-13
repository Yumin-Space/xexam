// Per-user AI+ feature permissions (v1.1, FP-1 / FP-3, P-9 .. P-12) and the one rule that decides every AI control
// (M-7, A-OWNER-15). Pure: no DOM, no i18n calls (it runs under `node --test`).
import type { Role } from '../types/api.ts';
import type { AiStatus } from '../state/ai.ts';

export type FeatureKey = 'chat' | 'hint' | 'explain' | 'summary' | 'generate' | 'import';

/** The six features in FP-1 order (the order of the checkboxes and of every list of names) */
export const FEATURE_KEYS: readonly FeatureKey[] = ['chat', 'hint', 'explain', 'summary', 'generate', 'import'];

export type Features = Record<FeatureKey, boolean>;

export const ALL_ON: Features = { chat: true, hint: true, explain: true, summary: true, generate: true, import: true };

/** The two groups of the AI+ users table (P-12): line 1 and the "AI 题库：" line */
export const FEATURE_GROUPS: { main: readonly FeatureKey[]; banks: readonly FeatureKey[] } = {
  main: ['chat', 'hint', 'explain', 'summary'],
  banks: ['generate', 'import'],
};

/** The features that are off, in FP-1 order */
export function offList(f: Features): FeatureKey[] {
  return FEATURE_KEYS.filter((k) => !f[k]);
}

/**
 * The `features` object of the status / a user row. Missing or malformed → everything on; a missing key (an older
 * ai-service has no `import`) counts as on; only a real `false` switches a feature off.
 */
export function readFeatures(json: unknown): Features {
  if (typeof json !== 'object' || json === null || Array.isArray(json)) return { ...ALL_ON };
  const o = json as Record<string, unknown>;
  const out = { ...ALL_ON };
  for (const k of FEATURE_KEYS) if (o[k] === false) out[k] = false;
  return out;
}

export function sameFeatures(a: Features, b: Features): boolean {
  return FEATURE_KEYS.every((k) => a[k] === b[k]);
}

// ---------------------------------------------------------------- the visibility rule of every AI control

/** Why AI cannot be used now although the user has the AI+ feature (the BT-2 reasons) */
export type UnavailableReason = 'loading' | 'off' | 'down' | 'unavailable';

/**
 * hidden      — no AI+ feature, or a bank control without the bank-management permission: not rendered at all
 * paused      — AI+ granted but paused: shown greyed with the 无法使用 tag and the paused tip
 * unavailable — AI is off / not reachable / still loading: greyed with the 无法使用 tag (loading: no tag)
 * featureOff  — that feature is unticked: shown greyed with the 无法使用 tag and `ai.err.featureOff`
 * ok          — usable
 */
export type ControlView =
  | { view: 'hidden' }
  | { view: 'paused' }
  | { view: 'unavailable'; reason: UnavailableReason }
  | { view: 'featureOff' }
  | { view: 'ok' };

export interface ControlContext {
  role: Role;
  /** The user has the AI+ feature (the session's `aiAllowed`; always true for administrators) */
  service: boolean;
  /** The bank-management permission (administrators always qualify) */
  canManageBanks: boolean;
  status: AiStatus;
}

/**
 * M-7: decides one control. Usable only when AI+ is granted, not paused and the feature is ticked (plus, for the bank
 * controls, the bank-management permission). Administrators have every feature but see nothing while AI+ is not available.
 */
export function aiControlView(feature: FeatureKey, needsBanks: boolean, ctx: ControlContext): ControlView {
  const admin = ctx.role === 'admin';
  if (!admin && !ctx.service) return { view: 'hidden' };
  if (needsBanks && !admin && !ctx.canManageBanks) return { view: 'hidden' };
  const st = ctx.status;
  if (st.state === 'loading') return admin ? { view: 'hidden' } : { view: 'unavailable', reason: 'loading' };
  if (st.state === 'unavailable') return admin ? { view: 'hidden' } : { view: 'unavailable', reason: 'down' };
  if (!admin && st.suspended) return { view: 'paused' };
  if (!st.enabled) {
    if (admin) return { view: 'hidden' };
    return { view: 'unavailable', reason: st.why === 'off' ? 'off' : st.why === 'link' ? 'down' : 'unavailable' };
  }
  if (!admin && readFeatures(st.features)[feature] === false) return { view: 'featureOff' };
  return { view: 'ok' };
}
