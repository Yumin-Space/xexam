// Per-user AI+ feature permissions (v1.1, FP-1 / FP-3, P-9 .. P-12) and the one rule that decides every AI control
// (M-7, A-OWNER-15). Pure: no DOM, no i18n calls (it runs under `node --test`).
import type { Role } from '../types/api.ts';
import type { AiStatus } from '../state/ai.ts';

export type FeatureKey = 'chat' | 'hint' | 'explain' | 'summary' | 'generate' | 'import';

/** The six features in FP-1 order (the order of the checkboxes and of every list of names) */
export const FEATURE_KEYS: readonly FeatureKey[] = ['chat', 'hint', 'explain', 'summary', 'generate', 'import'];

export type Features = Record<FeatureKey, boolean>;

export const ALL_ON: Features = { chat: true, hint: true, explain: true, summary: true, generate: true, import: true };

/**
 * The two groups of the AI+ users table (P-12) and of the global switches: line 1 and the "AI 题库：" line. One entry is one
 * box / switch. The per-user boxes keep hint and analysis (`explain`) as ONE item, shown under `hint`: it writes both keys.
 * (The global switches of AI settings are independent per feature since v1.9.)
 */
export const FEATURE_GROUPS: { main: readonly FeatureKey[]; banks: readonly FeatureKey[] } = {
  main: ['chat', 'hint', 'summary'],
  banks: ['generate', 'import'],
};

/** The keys one box / switch writes: `hint` and `explain` always together, every other key alone */
export function pairedKeys(k: FeatureKey): readonly FeatureKey[] {
  return k === 'hint' || k === 'explain' ? ['hint', 'explain'] : [k];
}

/** The state of one box / switch: the hint + analysis item is on when either key is on (the server keeps them equal) */
export function itemOn(f: Partial<Record<FeatureKey, boolean>>, k: FeatureKey): boolean {
  return pairedKeys(k).some((x) => f[x] !== false);
}

/** `f` with one box / switch set: both keys of the hint + analysis item change together */
export function withItem<T extends Partial<Record<FeatureKey, boolean>>>(f: T, k: FeatureKey, on: boolean): T {
  const out = { ...f };
  for (const x of pairedKeys(k)) (out as Partial<Record<FeatureKey, boolean>>)[x] = on;
  return out;
}

/** The catalog key of a box / switch label: `ai.feature.study` ("AI 提示+解析") for the hint + analysis item */
export function featureLabelKey(k: FeatureKey, short = false): string {
  return `ai.feature.${k === 'hint' ? 'study' : k}${short ? '.short' : ''}`;
}

/** The label keys of a list of switched-off features: hint and analysis, both off, are named once as the one item */
export function offLabelKeys(off: readonly FeatureKey[]): string[] {
  const both = off.includes('hint') && off.includes('explain');
  return off.filter((k) => !(both && k === 'explain')).map((k) => (both && k === 'hint' ? featureLabelKey('hint') : `ai.feature.${k}`));
}

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
 * featureOff  — that feature is switched off (a per-user box, or since v1.9 the global switch, which also applies to
 *                administrators): shown greyed with the 无法使用 tag and `ai.err.featureOff` (`ai.err.featureOffAdmin` for admins)
 * ok          — usable
 */
export type ControlView =
  | { view: 'hidden' }
  | { view: 'paused' }
  | { view: 'unavailable'; reason: UnavailableReason }
  | { view: 'featureOff'; admin?: boolean }
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
 * controls, the bank-management permission). Per-user boxes never limit administrators, but the global feature switches
 * (v1.9) do; they see nothing while AI+ is not available.
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
  if (readFeatures(st.features)[feature] === false) return admin ? { view: 'featureOff', admin: true } : { view: 'featureOff' };
  return { view: 'ok' };
}
