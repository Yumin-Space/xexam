// The AI+ badge (v0.8): an original slanted orange-yellow tag with a soft glow (styles in main.css, .ai-badge). The text
// is the same in every language. Which badge an account shows is decided by domain/badge-state.ts (owner's three states):
//   · on   — bright: the user can use AI now; screen readers hear its label ("AI+ is on");
//   · off  — dark (.ai-badge.off, the same text and layout without the orange): the user has the AI+ service but cannot use
//            AI now; the tip says why (paused, switched off, not reachable);
//   · none — no badge (aiBadge returns null).
// v1.1 (BT-1 .. BT-6): EVERY badge has a tip (domain/badge-state.ts `badgeTip` picks the key). Standalone badges are focusable
// and toggle their tip on a tap; a badge inside a button (the header's account button) shows it on hover (the button itself
// gets the tip through holdTip on touch); a badge inside an interactive head or a greyed bar stays decorative for screen
// readers and gets hover + tap-and-hold. The AI+题集 marker of AI-generated banks is `aiSetBadge` (GB-6).
import { aiSetTip, aiSetWordKey, minuteText } from '../domain/ai-gen.ts';
import type { FeatureKey } from '../domain/ai-features.ts';
import { badgeTip, type BadgeReason, type BadgeState, type BadgeViewer } from '../domain/badge-state.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import type { BankAi } from '../types/api.ts';
import { h } from '../utils/dom.ts';
import { holdTip, tipEl, withTip } from './tooltip.ts';

/** The text on the badge (a constant, not a catalog key); the "+" is drawn as a separate symbol a little to the right */
export const AI_BADGE_TEXT = 'AI+';

/** "AI" and the "+" symbol as two parts, so the symbol can sit apart from the letters (owner's request) */
function badgeText(word?: string): HTMLElement {
  const t0 = h('span', { class: 'ai-badge-t' }, AI_BADGE_TEXT.slice(0, -1), h('span', { class: 'ai-badge-plus' }, AI_BADGE_TEXT.slice(-1)));
  return word ? h('span', { class: 'ai-badge-w' }, t0, word) : t0;
}

/** The names of the features that are off, joined like any list of the UI language */
function offNames(off: readonly FeatureKey[]): string {
  return off.map((k) => t(`ai.feature.${k}` as MsgKey)).join(t('common.listSep'));
}

/** The tip text of a badge state for a viewer (BT-2); null when the state has no badge */
export function badgeTipText(state: BadgeState, viewer: BadgeViewer = 'own', off: readonly FeatureKey[] = []): string | null {
  const tip = badgeTip(state, viewer, off);
  if (!tip) return null;
  return t(tip.key as MsgKey, tip.off.length > 0 ? { list: offNames(tip.off) } : undefined);
}

/** The reason of a dark badge, in words (its tip; also the {reason} of the account button's label, the Settings and AI Chat lines) */
export const badgeReasonText = (reason: BadgeReason, admin = false): string =>
  badgeTipText({ kind: 'off', reason }, admin ? 'admin' : 'own') ?? '';

export interface AiBadgeOptions {
  /** The badge sits inside a button: not focusable (the button's label carries the state); its tip shows on hover */
  inButton?: boolean;
  /** The badge sits inside an interactive head or bar: decorative for screen readers, tip on hover and tap-and-hold */
  held?: boolean;
  /** An administrator is looking at another account (User management, AI settings): the tips use the admin wording */
  admin?: boolean;
  /** The features that are off (bright badge only): the tip names them (BT-2 `onSome`) */
  off?: readonly FeatureKey[];
}

let tipSeq = 0;

/** The badge for a state of domain/badge-state.ts: bright, dark with its reason, or null for 'none'; always with its tip */
export function aiBadge(state: BadgeState, o: AiBadgeOptions = {}): HTMLElement | null {
  const text = badgeTipText(state, o.admin ? 'admin' : 'own', o.off ?? []);
  if (state.kind === 'none' || text == null) return null;
  const on = state.kind === 'on';
  tipSeq += 1;
  const id = `ai-badge-tip-${tipSeq}`;
  const standalone = !o.inButton && !o.held;
  const badge = h('span', {
    class: 'ai-badge' + (on ? '' : ' off'), 'data-reason': state.kind === 'off' ? state.reason : null,
    role: o.held ? null : 'img', 'aria-hidden': o.held ? 'true' : null,
    'aria-label': o.held ? null : t(on ? 'ai.badge.label' : 'ai.badge.offLabel'),
    tabindex: standalone ? '0' : null, 'aria-describedby': standalone ? id : null,
  }, badgeText());
  if (o.held) holdTip(badge, text);
  // a tap toggles the tip of a standalone badge (touch phones have no hover and no keyboard focus ring); a tap elsewhere or Esc
  // closes it (components/tooltip.ts `.tip-tap`)
  return withTip(badge, tipEl(id, text), 'ai-badge-tip' + (standalone ? ' tip-tap' : ''));
}

/**
 * The AI+ set marker (GB-6, IM-8): ONE badge with the AI+ look whose text is "AI+" followed by the word (练题集 / 错题集 / 导入题集 by origin),
 * always bright (it says where the bank came from, not what the viewer may do), a label with no action, standalone tip
 * behaviour: who made it and when (or the file it came from), then the line that the answers are not checked.
 */
export function aiSetBadge(ai: BankAi): HTMLElement {
  const tip = aiSetTip(ai);
  tipSeq += 1;
  const id = `ai-badge-tip-${tipSeq}`;
  const first = t(tip.key as MsgKey, { ...tip.params });
  const tipNode = h('span', { class: 'tip', role: 'tooltip', id }, first, h('br'), t('aigen.badge.tipCheck'));
  const badge = h('span', {
    class: 'ai-badge ai-set', role: 'img', tabindex: '0', 'aria-label': t('aigen.badge.label'), 'aria-describedby': id,
  }, badgeText(t(aiSetWordKey(ai) as MsgKey)));
  return withTip(badge, tipNode, 'ai-badge-tip ai-set-tip tip-tap');
}

export { minuteText, tipEl };
