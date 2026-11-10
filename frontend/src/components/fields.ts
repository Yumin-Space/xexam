// Form fields shared by the sign-in pages, Settings > Account and User management: a labelled field with an optional hint,
// and for passwords a show / hide button and a Caps Lock hint. The caller keeps the typed value (onInput), because a page is
// rebuilt on every render and the field must come back with the same text.
import { t } from '../i18n/index.ts';
import { h } from '../utils/dom.ts';
import { icon } from './shell.ts';

const ICON_EYE = [{ d: 'M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z' }, { cx: 12, cy: 12, r: 2.8 }];
const ICON_EYE_OFF = [
  { d: 'M3.5 3.5l17 17' },
  { d: 'M10.3 5.7A9 9 0 0 1 12 5.5c6 0 9.5 6.5 9.5 6.5a17 17 0 0 1-3 3.8' },
  { d: 'M6.6 6.8C4 8.6 2.5 12 2.5 12S6 18.5 12 18.5a8.8 8.8 0 0 0 4.4-1.2' },
  { d: 'M9.9 9.9a3 3 0 0 0 4.2 4.2' },
];

export interface FieldOptions {
  id: string;
  label: string;
  value?: string;
  /** `password` fields get the show / hide button and the Caps Lock hint */
  type?: 'text' | 'password';
  autocomplete?: string;
  /** A short line under the field (also its accessible description) */
  hint?: string;
  maxlength?: number;
  disabled?: boolean;
  /** The current text, on every keystroke */
  onInput?: (value: string) => void;
  /** Password fields: whether the text is shown, and when the button changes it */
  shown?: boolean;
  onShow?: (shown: boolean) => void;
}

export interface Field {
  el: HTMLElement;
  input: HTMLInputElement;
}

/** A labelled field. Usernames are typed as they are: no auto-capitalisation, correction or spell check. */
export function field(o: FieldOptions): Field {
  const password = o.type === 'password';
  const hintId = o.hint ? `${o.id}-hint` : null;
  const capsId = password ? `${o.id}-caps` : null;
  const input = h('input', {
    id: o.id, name: o.id, type: password && !o.shown ? 'password' : 'text', value: o.value ?? '',
    autocomplete: o.autocomplete ?? 'off', autocapitalize: 'none', autocorrect: 'off', spellcheck: 'false',
    maxlength: o.maxlength ? String(o.maxlength) : null, disabled: !!o.disabled,
    'aria-describedby': [hintId, capsId].filter(Boolean).join(' ') || null,
    oninput: () => o.onInput?.(input.value),
  });
  if (!password) {
    return {
      el: h('div', { class: 'fld' }, h('label', { class: 'fld-l', for: o.id }, o.label), input, o.hint ? h('p', { class: 'fld-hint', id: hintId }, o.hint) : null),
      input,
    };
  }

  const caps = h('p', { class: 'fld-caps', id: capsId, hidden: true }, t('auth.caps'));
  const onKey = (e: KeyboardEvent): void => {
    if (typeof e.getModifierState === 'function') caps.hidden = !e.getModifierState('CapsLock');
  };
  input.addEventListener('keydown', onKey);
  input.addEventListener('keyup', onKey);
  input.addEventListener('blur', () => {
    caps.hidden = true;
  });

  const toggle = h('button', {
    class: 'fld-eye', type: 'button', disabled: !!o.disabled, 'aria-controls': o.id,
    'aria-pressed': String(!!o.shown), 'aria-label': t('auth.show'), title: t(o.shown ? 'auth.hide' : 'auth.show'),
    onclick: () => {
      const show = input.type === 'password';
      input.type = show ? 'text' : 'password';
      toggle.setAttribute('aria-pressed', String(show));
      toggle.title = t(show ? 'auth.hide' : 'auth.show');
      toggle.replaceChildren(icon(show ? ICON_EYE_OFF : ICON_EYE));
      o.onShow?.(show);
    },
  }, icon(o.shown ? ICON_EYE_OFF : ICON_EYE));

  return {
    el: h('div', { class: 'fld' },
      h('label', { class: 'fld-l', for: o.id }, o.label),
      h('div', { class: 'fld-pw' }, input, toggle),
      o.hint ? h('p', { class: 'fld-hint', id: hintId }, o.hint) : null,
      caps),
    input,
  };
}

/** Number of characters as the backend counts them (Unicode code points, not UTF-16 units) */
export const charCount = (s: string): number => [...s].length;

/** The backend's password rule: 8 to 128 characters */
export const passwordLengthOk = (s: string): boolean => charCount(s) >= 8 && charCount(s) <= 128;

/** The backend's username rule (after trimming): 1 to 40 of ASCII letters, digits and . _ - @ */
export const usernameOk = (s: string): boolean => /^[A-Za-z0-9._@-]{1,40}$/.test(s.trim());
