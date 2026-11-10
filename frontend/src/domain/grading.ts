// 判分与错题本规则（纯函数，不碰 DOM、不碰网络）。
//
// 说明：规格要求"前端不含权威业务规则"，但旧版的判分与错题本规则就在浏览器里，服务端只存 {id, correct}。
// 为了保持 API 向后兼容，这里沿用旧行为，并把它隔离成纯函数 + 金标准测试（grading.test.ts）。
// 这是对规格的有意偏离，见 docs/migration-plan.md 的 Q3。
import type { Mode, Progress, Question, QuestionStat } from '../types/api.ts';

/** 作答。单选 / 多选：选中的字母；热点题：每个小项选了第几个（没选是 null）。 */
export type Answer = Array<string | number | null> | undefined | null;

/** 0 未答；1 部分（只有热点题会有）；2 已答完 */
export type AnswerState = 0 | 1 | 2;

export function stateOf(q: Pick<Question, 'kind' | 'items'>, a: Answer): AnswerState {
  if (!Array.isArray(a)) return 0;
  if (q.kind === 'hotspot') {
    const c = a.filter((x) => x != null).length;
    return c === 0 ? 0 : c === q.items.length ? 2 : 1;
  }
  return a.length ? 2 : 0;
}

export function grade(q: Pick<Question, 'kind' | 'answer'>, a: Answer): boolean {
  if (!Array.isArray(a)) return false;
  const right = q.answer as Array<string | number>;
  if (q.kind === 'hotspot') {
    return a.length === right.length && right.every((x, i) => a[i] === x);
  }
  const mine = [...a].map(String).sort();
  const want = [...right].map(String).sort();
  return mine.length === want.length && mine.every((x, i) => x === want[i]);
}

/** 错题本：答错过，且连续答对的次数还不到 masterStreak。 */
export function inNotebook(stat: QuestionStat | undefined | null, masterStreak: number): boolean {
  return !!stat && stat.wrong > 0 && stat.streak < masterStreak;
}

export function masterStreakOf(p: Progress | null | undefined): number {
  return (p && p.masterStreak) || 2;
}

/** 出题范围：选中的工作表里的题；"错题本"模式只留错题本里的。 */
export function poolOf(
  questions: Question[],
  selected: ReadonlySet<string>,
  mode: Mode,
  progress: Progress | null | undefined,
): Question[] {
  const ms = masterStreakOf(progress);
  return questions.filter(
    (q) => selected.has(q.sheet) && (mode !== 'weak' || inNotebook(progress?.questions?.[q.id], ms)),
  );
}

/**
 * 百分比：四舍六入五成双，和服务端（Python 的 round）一致，
 * 这样成绩页和学习记录里的数字不会差 1。用整数算，避免浮点误差。
 */
export function percentOf(good: number, total: number): number {
  if (total <= 0) return 0;
  const num = 100 * good;
  const q = Math.floor(num / total);
  const r2 = 2 * (num - q * total);
  if (r2 < total) return q;
  if (r2 > total) return q + 1;
  return q % 2 === 0 ? q : q + 1;
}

/** Fisher–Yates */
export function shuffle<T>(arr: readonly T[], random: () => number = Math.random): T[] {
  const a = arr.slice();
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    const tmp = a[i] as T;
    a[i] = a[j] as T;
    a[j] = tmp;
  }
  return a;
}
