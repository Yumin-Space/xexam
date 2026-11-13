// Which AI+ badge an account shows (v0.8, owner's three states; contracts §3.7). One rule for every place that shows a
// badge: the header and the menu's account area (the signed-in user), Settings › Account, the rows of User management
// and the "Users with AI+" table of AI settings (administrators looking at others). No DOM (it runs under `node --test`);
// components/ai-badge.ts draws the result.
//   · none   — no badge: a plain user without the AI+ service (User management switch off); an administrator while AI+ is
//              not available (administrators need no switch and are never suspended, so they never get the dark badge);
//   · off    — the dark badge: the user has the AI+ service but cannot use AI now, because an administrator suspended their
//              AI+ function in AI settings (`suspended`, checked first), or because AI+ is not available (the AI+ service
//              is off in AI settings, or ai-service is not connected: `unavailable`). The tooltip says which;
//   · on     — the bright badge: the user can use AI now.
import type { Role } from '../types/api.ts';
import type { FeatureKey } from './ai-features.ts';

// v1.1 (BT-2): the dark badge names its cause.
//   suspended   - an administrator paused the user's AI+ function (checked first)
//   off         - ai-service answers, but its AI+ service is not effective (switched off in AI settings, not tested, key problem)
//   down        - ai-service cannot be reached, or the main service has had no report from it for 60 s
//   unavailable - the cause is not known (an older ai-service without `why`)
export type BadgeReason = 'suspended' | 'off' | 'down' | 'unavailable';
export type BadgeViewer = 'own' | 'admin';

export type BadgeState = { kind: 'none' } | { kind: 'off'; reason: BadgeReason } | { kind: 'on' };

export interface BadgeInput {
  role: Role;
  /** The user has the AI+ service (User management; `users.ai_enabled`). Ignored for administrators */
  service: boolean;
  /** Their AI+ function is suspended in AI settings. Ignored for administrators */
  suspended: boolean;
  /** AI+ is available: ai-service is connected and the AI+ service is on in AI settings */
  available: boolean;
  /** v1.1: the cause of a dark badge when `available` is false; default `unavailable` */
  unavailableReason?: 'off' | 'down';
}

export function badgeState(i: BadgeInput): BadgeState {
  if (i.role === 'admin') return i.available ? { kind: 'on' } : { kind: 'none' };
  if (!i.service) return { kind: 'none' };
  if (i.suspended) return { kind: 'off', reason: 'suspended' };
  if (!i.available) return { kind: 'off', reason: i.unavailableReason ?? 'unavailable' };
  return { kind: 'on' };
}

/** What the signed-in user's own badge needs from the AI status of state/ai.ts (GET /api/ai/status) */
export interface OwnAiStatus {
  state: 'loading' | 'unavailable' | 'ready';
  /** AI+ is available (the AI service's `enabled`: the AI+ service on and ai-service connected to the main service) */
  enabled?: boolean;
  /** The user's AI+ function is suspended by an administrator */
  suspended?: boolean;
  /** v1.1: why AI is not enabled: `off` (the AI+ service is not effective), `link` (no report for 60 s), null / absent = unknown */
  why?: 'off' | 'link' | null;
}

/**
 * The signed-in user's own badge (header, menu, Settings › Account): the AI+ service from the session (`aiAllowed`,
 * always true for administrators), suspended and available from the AI status. Nothing while the first AI status is still
 * being read (after signing in or a reload), so the dark badge never flashes up before the bright one; an AI service that
 * cannot be reached counts as "not available".
 */
export function ownBadgeState(user: { role: Role; aiAllowed: boolean } | null, st: OwnAiStatus): BadgeState {
  if (!user || st.state === 'loading') return { kind: 'none' };
  const ready = st.state === 'ready';
  const reason = !ready ? 'down' : st.why === 'off' ? 'off' : st.why === 'link' ? 'down' : undefined;
  return badgeState({
    role: user.role, service: user.aiAllowed, suspended: ready && st.suspended === true, available: ready && st.enabled === true,
    ...(reason ? { unavailableReason: reason } : {}),
  });
}

/** The tip of a badge: a catalog key; `off` = the features that are off (bright states only), the component translates their names */
export interface BadgeTip {
  key: string;
  off: FeatureKey[];
}

/**
 * BT-2 table: which tip a badge gets for its state and viewer (`own` = the user's own badge, `admin` = an administrator
 * looking at another account). Pure: keys only.
 */
export function badgeTip(b: BadgeState, viewer: BadgeViewer, off: readonly FeatureKey[] = []): BadgeTip | null {
  const admin = viewer === 'admin';
  if (b.kind === 'none') return null;
  if (b.kind === 'on') {
    if (off.length > 0) return { key: admin ? 'ai.badge.tip.adminOnSome' : 'ai.badge.tip.onSome', off: [...off] };
    return { key: admin ? 'ai.badge.tip.adminOn' : 'ai.badge.tip.on', off: [] };
  }
  switch (b.reason) {
    case 'suspended': return { key: admin ? 'ai.badge.tip.adminSuspended' : 'ai.badge.tip.suspended', off: [] };
    case 'off': return { key: admin ? 'ai.badge.tip.adminOff' : 'ai.badge.tip.off', off: [] };
    case 'down': return { key: admin ? 'ai.badge.tip.adminDown' : 'ai.badge.tip.down', off: [] };
    default: return { key: admin ? 'ai.badge.unavailableAdmin' : 'ai.badge.unavailable', off: [] };
  }
}

/**
 * The cause of "AI+ is not available" for an administrator's view of other accounts (BT-2): from the state of ai-service as the
 * main service knows it (`S.aiService`): not connected -> `down`, connected but the AI+ service not on -> `off`, unknown -> none.
 */
export function serviceReason(svc: { connected: boolean; enabled: boolean } | null): 'off' | 'down' | undefined {
  if (!svc) return undefined;
  if (!svc.connected) return 'down';
  return svc.enabled ? undefined : 'off';
}

/** The same state as a short word, for tests and for data attributes */
export const badgeKey = (b: BadgeState): 'none' | 'on' | 'off-suspended' | 'off-off' | 'off-down' | 'off-unavailable' =>
  b.kind === 'off' ? `off-${b.reason}` : b.kind;
