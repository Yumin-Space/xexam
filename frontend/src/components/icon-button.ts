// Icon buttons (v1.0, spec SH-2): a button that shows a mini icon, with or without its text. The label is always the
// accessible name; without a visible label it is also the tip (mouse hover and keyboard focus on desktop, tap-and-hold on
// touch: components/tooltip.ts `holdTip`). The look (36 / 40 px visual, 44 px hit area on touch, tones, greyed state) is
// styles/icon-button.css.
//   · iconButton()  the button;
//   · shortLabel()  a full and a short text for tight rows (the full one stays the accessible name);
//   · withReason()  a control that cannot be used for a reason the user must be able to read (aria-disabled, never `disabled`).
import { t, type MsgKey } from '../i18n/index.ts';
import { h } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';
import '../styles/icon-button.css';
import { miniIcon, type MiniIcon } from './mini-icons.ts';
import { holdTip, tipEl, withTip } from './tooltip.ts';

export type IconTone = 'default' | 'danger' | 'ai' | 'quiet';

export interface IconButtonOptions {
  icon: MiniIcon;
  /** The accessible name (and the tip when no label is shown) */
  label: string;
  onClick?: (e: MouseEvent) => void;
  tone?: IconTone;
  /** Icon + text (desktop bars, wide layouts): the text is the name, so there is no aria-label */
  showLabel?: boolean;
  /** A toggle: sets aria-pressed */
  pressed?: boolean;
  /** The data-focus key (the page puts the focus back on this button after a redraw) */
  focusKey?: string;
  describedBy?: string;
}

/** `button.ibtn` with the icon (18 px visual); see the file header */
export function iconButton(o: IconButtonOptions): HTMLButtonElement {
  const tone = o.tone ?? 'default';
  const btn = h('button', {
    type: 'button',
    class: `ibtn ibtn-${tone}${o.showLabel ? ' ibtn-label' : ''}`,
    'aria-label': o.showLabel ? null : o.label,
    'aria-pressed': o.pressed === undefined ? null : String(o.pressed),
    'aria-describedby': o.describedBy ?? null,
    'data-focus': o.focusKey ?? null,
    onclick: o.onClick,
  }, miniIcon(o.icon, 'ibtn-ic'), o.showLabel ? h('span', { class: 'ibtn-lbl' }, o.label) : null);
  if (!o.showLabel) holdTip(btn, o.label);
  return btn;
}

/**
 * A full and a short text for tight rows: `span.l-full` + `span.l-short`. CSS shows the short one at `max-width: 400px`
 * (styles/icon-button.css); the short one is aria-hidden and the full one only hidden visually (clipped), so the
 * accessible name is always the full text. Put the pair inside the control: `h('button', {}, shortLabel(a, b))`.
 */
export function shortLabel(full: MsgKey, short: MsgKey): [HTMLSpanElement, HTMLSpanElement] {
  return [h('span', { class: 'l-full' }, t(full)), h('span', { class: 'l-short', 'aria-hidden': 'true' }, t(short))];
}

export interface ReasonOptions {
  /** Runs first on a tap; returning true means "handled" and no reason toast is shown (e.g. a re-check that freed the control) */
  beforeToast?: () => boolean;
  /** A-OWNER-65b: no toast; a tap, hover or focus shows the reason as a tip box (the tap-to-open tip of the job-card numbers) */
  tapTip?: boolean;
}

let reasonSeq = 0;

/**
 * A control that cannot be used right now, for a reason the user must be able to read: `aria-disabled="true"` (never
 * `disabled`, so it stays focusable and tappable), the reason as tooltip and as `aria-describedby`, and a tap shows the
 * reason as a toast. Returns the control wrapped in its tooltip (`span.has-tip.ibtn-reason`). The control must not carry its
 * action handler (a tap on an aria-disabled button still fires `click`): build it without `onClick`, then wrap it here.
 */
export function withReason(control: HTMLElement, text: string, opts: ReasonOptions = {}): HTMLSpanElement {
  reasonSeq += 1;
  const id = `reason-${reasonSeq}`;
  control.setAttribute('aria-disabled', 'true');
  control.setAttribute('aria-describedby', id);
  if (!opts.tapTip) {
    control.addEventListener('click', () => {
      if (opts.beforeToast?.()) return;
      toast(text);
    });
  }
  // An icon-only button already has a hold tip (its label): that tip now says the reason; the wrapper's own tip stays hidden
  // (aria-describedby still reads it)
  const own = control.classList.contains('ibtn') && !control.classList.contains('ibtn-label');
  if (own) holdTip(control, text);
  return withTip(control, tipEl(id, text), (own ? 'ibtn-reason ibtn-reason-own' : 'ibtn-reason') + (opts.tapTip && !own ? ' tip-tap' : ''));
}
