// 一道题在某种语言下该显示什么：纯函数，没有 DOM。答题页和成绩页共用，所以日语 / 中文 / 英语的规则只写一份。
//
// 规则：
//   · 日语永远可用（它是题目本身，题目 id、选项字母、热点题的答案都来自日语）；
//   · 中文 / 英语只有在这道题有对应的结构（选项字母、小项个数都能和日语对上）时才可用；
//   · 选的语言这道题没有，就退回日语 —— 不显示错位的内容。
import type { En, Question, Zh } from '../types/api.ts';

export type Lang = 'ja' | 'zh' | 'en';

export const LANGS: readonly Lang[] = ['ja', 'zh', 'en'];

export function isLang(v: unknown): v is Lang {
  return v === 'ja' || v === 'zh' || v === 'en';
}

/** 这道题能不能用这种语言显示 */
export function hasLang(q: Question, lang: Lang): boolean {
  if (lang === 'ja') return true;
  return lang === 'zh' ? !!q.zh : !!q.en;
}

/** 想要的语言这道题没有时退回日语 */
export function effectiveLang(q: Question, want: Lang): Lang {
  return hasLang(q, want) ? want : 'ja';
}

export interface Shown {
  /** 实际用的语言（HTML 的 lang 属性） */
  lang: Lang;
  stem: string;
  /** 单选 / 多选：第 i 个选项的文字 */
  optionText: (i: number) => string;
  /** 热点题：第 i 个小项的文字 */
  itemText: (i: number) => string;
  /** 热点题：第 i 个小项、第 j 个选项的文字：用这门语言那一列里写的选项（是 / 否、Yes / No……）；没写就用日语的 */
  choiceText: (i: number, j: number) => string;
  /** 热点题的选项实际用的语言（HTML 的 lang 属性）：这门语言写了选项就是它，否则是日语 */
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
    // 选项的位置就是答案的下标，三种语言里「第 j 个」是同一个意思；这门语言没写选项时退回日语（答案原文）
    choiceText: (i, j) => other?.choices?.[i]?.[j] || (q.items[i]?.choices[j] ?? ''),
    choiceLang: (i, j) => (other?.choices?.[i]?.[j] ? lang : 'ja'),
  };
}
