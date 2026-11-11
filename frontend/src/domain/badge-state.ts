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

export type BadgeReason = 'suspended' | 'unavailable';

export type BadgeState = { kind: 'none' } | { kind: 'off'; reason: BadgeReason } | { kind: 'on' };

export interface BadgeInput {
  role: Role;
  /** The user has the AI+ service (User management; `users.ai_enabled`). Ignored for administrators */
  service: boolean;
  /** Their AI+ function is suspended in AI settings. Ignored for administrators */
  suspended: boolean;
  /** AI+ is available: ai-service is connected and the AI+ service is on in AI settings */
  available: boolean;
}

export function badgeState(i: BadgeInput): BadgeState {
  if (i.role === 'admin') return i.available ? { kind: 'on' } : { kind: 'none' };
  if (!i.service) return { kind: 'none' };
  if (i.suspended) return { kind: 'off', reason: 'suspended' };
  if (!i.available) return { kind: 'off', reason: 'unavailable' };
  return { kind: 'on' };
}

/** What the signed-in user's own badge needs from the AI status of state/ai.ts (GET /api/ai/status) */
export interface OwnAiStatus {
  state: 'loading' | 'unavailable' | 'ready';
  /** AI+ is available (the AI service's `enabled`: the AI+ service on and ai-service connected to the main service) */
  enabled?: boolean;
  /** The user's AI+ function is suspended by an administrator */
  suspended?: boolean;
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
  return badgeState({
    role: user.role, service: user.aiAllowed, suspended: ready && st.suspended === true, available: ready && st.enabled === true,
  });
}

/** The same state as a short word, for tests and for data attributes */
export const badgeKey = (b: BadgeState): 'none' | 'on' | 'off-suspended' | 'off-unavailable' =>
  b.kind === 'off' ? `off-${b.reason}` : b.kind;
