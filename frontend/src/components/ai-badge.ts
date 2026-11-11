// The AI+ badge (v0.8): an original slanted orange-yellow tag with a soft glow (styles in main.css, .ai-badge). The text
// is the same in every language. Which badge an account shows is decided by domain/badge-state.ts (owner's three states):
//   · on   — bright: the user can use AI now; screen readers hear its label ("AI+ is on");
//   · off  — dark (.ai-badge.off, the same text and layout without the orange): the user has the AI+ service but cannot use
//            AI now; the shared tooltip says why (their AI+ function is suspended, or the AI+ service is not available).
//            It is focusable so keyboards reach the reason too, except inside a button (the header's account button), where
//            the button's own label carries the reason and the tooltip still shows on hover;
//   · none — no badge (aiBadge returns null).
import { t } from '../i18n/index.ts';
import type { BadgeReason, BadgeState } from '../domain/badge-state.ts';
import { h } from '../utils/dom.ts';
import { tipEl, withTip } from './tooltip.ts';

/** The text on the badge (a constant, not a catalog key); the "+" is drawn as a separate symbol a little to the right */
export const AI_BADGE_TEXT = 'AI+';

/** "AI" and the "+" symbol as two parts, so the symbol can sit apart from the letters (owner's request) */
function badgeText(): HTMLElement {
  return h('span', { class: 'ai-badge-t' }, AI_BADGE_TEXT.slice(0, -1), h('span', { class: 'ai-badge-plus' }, AI_BADGE_TEXT.slice(-1)));
}

/** The reason of a dark badge, in words (its tooltip; also the {reason} of the account button's label) */
export const badgeReasonText = (reason: BadgeReason, admin = false): string =>
  t(reason === 'suspended' ? 'ai.badge.suspended' : admin ? 'ai.badge.unavailableAdmin' : 'ai.badge.unavailable');

export interface AiBadgeOptions {
  /** The badge sits inside a button: the dark badge is not focusable (the button's label names the reason) */
  inButton?: boolean;
  /** An administrator is looking (User management): the "not available" tooltip may name the cause in their words */
  admin?: boolean;
}

let tipSeq = 0;

function offBadge(reason: BadgeReason, o: AiBadgeOptions): HTMLElement {
  tipSeq += 1;
  const id = `ai-badge-tip-${tipSeq}`;
  const badge = h('span', {
    class: 'ai-badge off', role: 'img', 'aria-label': t('ai.badge.offLabel'), 'data-reason': reason,
    tabindex: o.inButton ? null : '0', 'aria-describedby': o.inButton ? null : id,
  }, badgeText());
  // Outside a button the badge also toggles its tip on a tap (touch phones have no hover and no keyboard focus ring); a tap
  // elsewhere or Esc closes it again (components/tooltip.ts, `.tip-tap`)
  return withTip(badge, tipEl(id, badgeReasonText(reason, o.admin === true)), 'ai-badge-tip' + (o.inButton ? '' : ' tip-tap'));
}

const onBadge = (): HTMLElement => h('span', { class: 'ai-badge', role: 'img', 'aria-label': t('ai.badge.label') }, badgeText());

/** The badge for a state of domain/badge-state.ts: bright, dark with its reason, or null for 'none' */
export function aiBadge(state: BadgeState, o: AiBadgeOptions = {}): HTMLElement | null {
  if (state.kind === 'none') return null;
  return state.kind === 'on' ? onBadge() : offBadge(state.reason, o);
}
