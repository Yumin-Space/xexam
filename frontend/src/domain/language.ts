// What a question should display in a given language: pure functions, no DOM. Shared by the quiz / practice page and the
// review cards (result page, exam detail, wrong-answer review), so the English / Chinese / Japanese rules are written once.
//
// Rules:
//   · every question has a BASE language: the language of its own stem / options / items, the one its id, option letters and
//     hotspot answers come from. That is Japanese for the 3- and 4-column Excel formats and English for the six-column
//     format (v0.6.5, marked by `q.lang === 'en'`);
//   · the base language is always available; the other two are available only when the question carries a translation with
//     a matching structure (option count and item count line up with the base): Chinese in `q.zh` (every format), English in
//     `q.en` (4-column format), Japanese in `q.ja` (six-column format);
//   · a language the question lacks falls back to the base language - never show misaligned content;
//   · explanations: six-column questions have one per language in `q.explain`; older questions have one explanation, which
//     is shown whatever the language (see explainOf).
import type { ExplainText, Question, Zh } from '../types/api.ts';

export type Lang = 'ja' | 'zh' | 'en';

/** The order of the buttons of every question-language switch (section 13 of the v0.6.5 spec: English / 中文 / 日本語) */
export const SWITCH_ORDER: readonly Lang[] = ['en', 'zh', 'ja'];

/** What each switch button says, the same in every UI language: each language named in its own script */
export const LANG_LABEL: Readonly<Record<Lang, string>> = { en: 'English', zh: '中文', ja: '日本語' };

export function isLang(v: unknown): v is Lang {
  return v === 'ja' || v === 'zh' || v === 'en';
}

/** The HTML lang attribute of a question language (Chinese is Simplified Chinese: zh-CN, like the UI language) */
export function htmlLang(lang: Lang): string {
  return lang === 'zh' ? 'zh-CN' : lang;
}

/** The language the question's own text is in: English for six-column questions, Japanese for every older format */
export function baseLang(q: Pick<Question, 'lang'>): Lang {
  return q.lang === 'en' ? 'en' : 'ja';
}

type LangFields = Pick<Question, 'lang' | 'zh' | 'en' | 'ja'>;

/** The translation object of `lang`, or null for the base language and for a language the question lacks */
function translation(q: LangFields, lang: Lang): Zh | null {
  if (lang === baseLang(q)) return null;
  const t = lang === 'zh' ? q.zh : lang === 'en' ? q.en : q.ja;
  return t ?? null;
}

/** Whether this question can be shown in this language */
export function hasLang(q: LangFields, lang: Lang): boolean {
  return lang === baseLang(q) || !!translation(q, lang);
}

/** Falls back to the base language when the question lacks the wanted language */
export function effectiveLang(q: LangFields, want: Lang): Lang {
  return hasLang(q, want) ? want : baseLang(q);
}

export interface Shown {
  /** The language actually used (see htmlLang for the HTML lang attribute) */
  lang: Lang;
  stem: string;
  /** Single / multiple choice: the text of option i */
  optionText: (i: number) => string;
  /** Hotspot question: the text of item i */
  itemText: (i: number) => string;
  /**
   * Hotspot question: the text of choice j of item i: uses the choices written in this language's column (是 / 否, Yes / No,
   * ...); falls back to the base language's ones if none are written
   */
  choiceText: (i: number, j: number) => string;
  /** The language actually used for that hotspot choice: this language if it has its own choices, otherwise the base */
  choiceLang: (i: number, j: number) => Lang;
}

export function shown(q: Question, want: Lang): Shown {
  const base = baseLang(q);
  const lang = effectiveLang(q, want);
  const other = translation(q, lang);
  return {
    lang,
    stem: other ? other.stem : q.stem,
    optionText: (i) => (other?.options?.[i] ?? q.options[i]?.text ?? ''),
    itemText: (i) => (other?.items?.[i] ?? q.items[i]?.text ?? ''),
    // The position of a choice is the answer index, so "the j-th" means the same in all three languages; when this language
    // has no choices, fall back to the base language (the original answer text)
    choiceText: (i, j) => other?.choices?.[i]?.[j] || (q.items[i]?.choices[j] ?? ''),
    choiceLang: (i, j) => (other?.choices?.[i]?.[j] ? lang : base),
  };
}

/** One explanation text and the language it is written in (null = unknown: a 3- / 4-column question; no lang attribute) */
export interface LangText {
  text: string;
  lang: Lang | null;
}

export interface ShownExplain {
  explanation: LangText;
  mnemonic: LangText;
  note: LangText;
}

/** The order the backend fills the compatibility fields from (Chinese, else Japanese, else English) */
const COMPAT_ORDER: readonly Lang[] = ['zh', 'ja', 'en'];

/**
 * The explanation, memory aid and answer note of a question in a language (each field on its own):
 *   · six-column questions: that language's own text from `q.explain`; a field that language leaves empty falls back to
 *     another language's text in the order Chinese, Japanese, English (the same order as the compatibility fields), so
 *     nothing written in the workbook is hidden, and its lang attribute says which language it is;
 *   · older questions (no `explain`): their single explanation, for every language.
 */
export function explainOf(q: Pick<Question, 'explanation' | 'mnemonic' | 'note' | 'explain'>, want: Lang): ShownExplain {
  const field = (k: keyof ExplainText): LangText => {
    const ex = q.explain;
    if (!ex) return { text: q[k] ?? '', lang: null };
    const own = ex[want]?.[k];
    if (own) return { text: own, lang: want };
    for (const l of COMPAT_ORDER) {
      const v = ex[l]?.[k];
      if (v) return { text: v, lang: l };
    }
    // Nothing per language (should not happen when the compatibility field has text): show that field as it is
    return { text: q[k] ?? '', lang: null };
  };
  return { explanation: field('explanation'), mnemonic: field('mnemonic'), note: field('note') };
}
