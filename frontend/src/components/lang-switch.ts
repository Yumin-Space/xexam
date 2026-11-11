// The English / 中文 / 日本語 switch of a question (v0.6.5, section 13): the same component on the quiz and practice page and
// on every answers-and-explanations card (result page, exam detail, wrong-answer review).
//   · The three buttons always say "English", "中文", "日本語" (each language named in its own script, whatever the UI
//     language) and carry the matching lang attribute (en, zh-CN, ja), so screen readers pronounce them right.
//   · The group's accessible name and the tooltips stay in the UI language.
//   · A language the question lacks stays focusable and tappable (aria-disabled, not disabled): a tap says why in a toast,
//     because phones never show the title tooltip and a disabled button would silently ignore the tap.
import { LANG_LABEL, SWITCH_ORDER, baseLang, hasLang, htmlLang, type Lang } from '../domain/language.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import type { Question } from '../types/api.ts';
import { h } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';

/** The language's name in the UI language (the tooltip of an available button) */
const LANG_NAME: Record<Lang, MsgKey> = { en: 'quiz.lang.en', zh: 'quiz.lang.zh', ja: 'quiz.lang.ja' };
/** Why a language button is unavailable (the question lacks that language): its tooltip, and the toast shown when it is tapped */
const LANG_MISSING: Record<Lang, MsgKey> = { en: 'quiz.lang.noEn', zh: 'quiz.lang.noZh', ja: 'quiz.lang.noJa' };

export interface LangSwitchOptions {
  /** Prefix of the buttons' data-focus keys (the quiz page puts the focus back on the same button after a redraw) */
  focusKey?: string;
  /** Extra class of the group */
  className?: string;
}

/**
 * The switch for question `q`. `current` is the language shown (already the effective one: a language the question lacks
 * never appears pressed); `onPick` is called with an available language that is not the current one.
 */
export function langSwitch(q: Question, current: Lang, onPick: (lang: Lang) => void, opts: LangSwitchOptions = {}): HTMLElement {
  const base = baseLang(q);
  const button = (code: Lang): HTMLElement => {
    const missing = code === base || hasLang(q, code) ? null : t(LANG_MISSING[code]);
    return h('button', {
      class: 'lang-btn', type: 'button', lang: htmlLang(code),
      'aria-pressed': String(current === code),
      'data-focus': opts.focusKey ? `${opts.focusKey}-${code}` : null,
      'data-lang': code,
      'aria-disabled': missing ? 'true' : null,
      title: missing ?? t(LANG_NAME[code]),
      onclick: () => {
        if (missing) toast(missing);
        else if (code !== current) onPick(code);
      },
    }, LANG_LABEL[code]);
  };
  return h('div', { class: 'lang' + (opts.className ? ' ' + opts.className : ''), role: 'group', 'aria-label': t('quiz.lang.label') },
    SWITCH_ORDER.map(button));
}
