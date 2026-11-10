// What a question should display in a given language: pure functions, no DOM. Shared by the quiz page and the results page, so the Japanese / Chinese / English rules are written only once.
//
// Rules:
//   · Japanese is always available (it is the question itself; the question id, option letters and hotspot answers all come from Japanese);
//   · Chinese / English are available only when the question has a matching structure (option letters and item count line up with the Japanese);
//   · if the question lacks the chosen language, fall back to Japanese - never show misaligned content.
import type { En, Question, Zh } from '../types/api.ts';

export type Lang = 'ja' | 'zh' | 'en';

export const LANGS: readonly Lang[] = ['ja', 'zh', 'en'];

export function isLang(v: unknown): v is Lang {
  return v === 'ja' || v === 'zh' || v === 'en';
}

/** Whether this question can be shown in this language */
export function hasLang(q: Question, lang: Lang): boolean {
  if (lang === 'ja') return true;
  return lang === 'zh' ? !!q.zh : !!q.en;
}

/** Falls back to Japanese when the question lacks the wanted language */
export function effectiveLang(q: Question, want: Lang): Lang {
  return hasLang(q, want) ? want : 'ja';
}

export interface Shown {
  /** The language actually used (the HTML lang attribute) */
  lang: Lang;
  stem: string;
  /** Single / multiple choice: the text of option i */
  optionText: (i: number) => string;
  /** Hotspot question: the text of item i */
  itemText: (i: number) => string;
  /** Hotspot question: the text of choice j of item i: uses the choices written in this language's column (是 / 否, Yes / No, ...); falls back to the Japanese ones if none are written */
  choiceText: (i: number, j: number) => string;
  /** The language actually used for the hotspot choices (the HTML lang attribute): this language if it has its own choices, otherwise Japanese */
  choiceLang: (i: number, j: number) => Lang;
}

export function shown(q: Question, want: Lang): Shown {
  const lang = effectiveLang(q, want);
  const other: Zh | En | null = (lang === 'zh' ? q.zh : lang === 'en' ? q.en : null) ?? null;
  return {
    lang,
    stem: other ? other.stem : q.stem,
    optionText: (i) => (other?.options?.[i] ?? q.options[i]?.text ?? ''),
    itemText: (i) => (other?.items?.[i] ?? q.items[i]?.text ?? ''),
    // The position of a choice is the answer index, so "the j-th" means the same in all three languages; when this language has no choices, fall back to Japanese (the original answer text)
    choiceText: (i, j) => other?.choices?.[i]?.[j] || (q.items[i]?.choices[j] ?? ''),
    choiceLang: (i, j) => (other?.choices?.[i]?.[j] ? lang : 'ja'),
  };
}
