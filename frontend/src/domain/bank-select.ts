// The checked banks and sheets of Bank management (v1.1, BL-3): what a deletion of the selection means. Pure: no DOM.
// A bank whose every sheet is checked goes whole (the bank route); a partly checked bank loses only its checked sheets (the
// sheet route, which the server refuses for AI banks and for "all sheets"); an AI bank has only its one sheet, so it can only
// go whole.

export interface SelBank {
  id: number;
  /** An AI bank: its one sheet is only part of the bank */
  ai: boolean;
  /** Every sheet name of the bank (retired-only sheets included) */
  sheets: readonly string[];
}

/** The key of one checked sheet */
export const sheetSelKey = (bank: number, sheet: string): string => `${bank}\u0000${sheet}`;

export interface DeletionPlan {
  /** Banks that go whole, in list order */
  banks: number[];
  /** Banks that lose only the named sheets, in list order (the sheet names in the bank's own order) */
  parts: Array<{ id: number; sheets: string[] }>;
}

/** What deleting the checked sheets does */
export function deletionPlan(keys: Iterable<string>, banks: readonly SelBank[]): DeletionPlan {
  const set = new Set(keys);
  const plan: DeletionPlan = { banks: [], parts: [] };
  for (const b of banks) {
    const picked = b.sheets.filter((s) => set.has(sheetSelKey(b.id, s)));
    if (picked.length === 0) continue;
    if (picked.length === b.sheets.length || b.ai) plan.banks.push(b.id);
    else plan.parts.push({ id: b.id, sheets: picked });
  }
  return plan;
}

/** How many sheets (of the listed banks) are checked */
export function selectionCount(keys: Iterable<string>, banks: readonly SelBank[]): number {
  const set = new Set(keys);
  let n = 0;
  for (const b of banks) for (const s of b.sheets) if (set.has(sheetSelKey(b.id, s))) n += 1;
  return n;
}

/** Some but not all sheets of a bank are checked (the bank box shows the dash) */
export function partlySelected(keys: Iterable<string>, bank: SelBank): boolean {
  const set = new Set(keys);
  const n = bank.sheets.filter((s) => set.has(sheetSelKey(bank.id, s))).length;
  return n > 0 && n < bank.sheets.length;
}
