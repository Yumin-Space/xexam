// Preview warnings of a six-column import (v0.7). English is authoritative in that layout: when the Chinese answer (column E)
// or the Japanese answer (column F) of a row differs from the English one (column D), the row is still imported with the
// English answer and the backend lists one warning per differing column, per sheet ("warnings": [{row, reason}], capped like
// the unreadable rows, with "warningsMore" / "warningsCount"; the plan's top-level "warnings" is the total).
// Pure rules, no DOM: the Question banks page turns this into a calm callout ("Sheet 1, row 12: the Chinese answer differs").
import type { SheetPlan, SkippedRow } from '../types/api.ts';

/**
 * The backend's warning reasons (backend/src/parser/question.rs WARN6_ZH_DIFFERS / WARN6_JA_DIFFERS; the test checks they
 * are still there). Kept in Chinese like every backend message; the page names the language instead of showing them.
 */
export const WARN_ZH_DIFFERS = 'E 列的中文答案和 D 列的英文答案不一致（已按英文答案导入）';
export const WARN_JA_DIFFERS = 'F 列的日文答案和 D 列的英文答案不一致（已按英文答案导入）';

/** Which answer column differs from the English one */
export type DiffLang = 'zh' | 'ja';

export interface RowWarning {
  row: number;
  /** The languages whose answer differs, Chinese first */
  langs: DiffLang[];
  /** Reasons this version does not know (shown as the server wrote them, translated where possible) */
  other: string[];
}

export interface SheetWarnings {
  sheet: string;
  /** One entry per row, in row order */
  rows: RowWarning[];
  /** Warnings of this sheet the preview does not list (over the backend's cap) */
  more: number;
}

export function diffLangOf(reason: string): DiffLang | null {
  if (reason === WARN_ZH_DIFFERS) return 'zh';
  if (reason === WARN_JA_DIFFERS) return 'ja';
  return null;
}

/** The listed warnings of one sheet, one entry per row (a row whose Chinese and Japanese answers both differ has two warnings) */
export function rowWarnings(list: readonly SkippedRow[]): RowWarning[] {
  const byRow = new Map<number, RowWarning>();
  for (const w of list) {
    let r = byRow.get(w.row);
    if (!r) {
      r = { row: w.row, langs: [], other: [] };
      byRow.set(w.row, r);
    }
    const lang = diffLangOf(w.reason);
    if (lang) {
      if (!r.langs.includes(lang)) r.langs.push(lang);
    } else if (!r.other.includes(w.reason)) {
      r.other.push(w.reason);
    }
  }
  const rows = [...byRow.values()].sort((a, b) => a.row - b.row);
  for (const r of rows) r.langs.sort((a, b) => (a === b ? 0 : a === 'zh' ? -1 : 1));
  return rows;
}

/** The sheets of a preview that have warnings, in sheet order (sheets without any are left out) */
export function sheetWarnings(sheets: readonly Pick<SheetPlan, 'name' | 'warnings' | 'warningsMore'>[]): SheetWarnings[] {
  return sheets.flatMap((s) => {
    const list = s.warnings ?? [];
    const more = Math.max(0, s.warningsMore ?? 0);
    if (!list.length && !more) return [];
    return [{ sheet: s.name, rows: rowWarnings(list), more }];
  });
}

/**
 * How many answers differ in the whole file: the plan's top-level count when the backend sends it, else the sum of the
 * sheets' counts (listed + not listed). 0 = no callout.
 */
export function warningTotal(plan: { warnings?: number; sheets: readonly Pick<SheetPlan, 'warnings' | 'warningsMore' | 'warningsCount'>[] }): number {
  if (typeof plan.warnings === 'number' && plan.warnings > 0) return plan.warnings;
  return plan.sheets.reduce((n, s) => n + (s.warningsCount ?? (s.warnings?.length ?? 0) + Math.max(0, s.warningsMore ?? 0)), 0);
}
