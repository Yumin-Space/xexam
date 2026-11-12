// The About page's newest version shows its lead paragraph and its first three bullets, then "Show all (n)" (AB-1). Pure, so it
// runs under `node --test`. The changelog files themselves are not touched: this only decides what is drawn at first.
import type { Block } from './changelog.ts';

/** How many top-level bullets of the newest version show before "Show all" */
export const SHOWN_BULLETS = 3;

export interface FoldedEntry {
  /** The blocks drawn while the entry is folded: the paragraphs before the first list, then the first `max` bullets */
  shown: Block[];
  /** All top-level bullets of the entry (the n of "Show all (n)") */
  total: number;
  /** Something is left out, so the entry needs the button */
  folded: boolean;
}

export function foldEntry(bs: readonly Block[], max = SHOWN_BULLETS): FoldedEntry {
  const shown: Block[] = [];
  let seenList = false;
  let used = 0;
  let total = 0;
  let folded = false;
  for (const b of bs) {
    if (b.kind === 'list') total += b.items.length;
  }
  for (const b of bs) {
    if (b.kind === 'para') {
      // The lead: paragraphs before the first list; a paragraph after the bullets is part of the rest
      if (seenList) folded = true;
      else shown.push(b);
      continue;
    }
    seenList = true;
    const room = max - used;
    if (room <= 0) {
      folded = true;
    } else if (b.items.length <= room) {
      shown.push(b);
      used += b.items.length;
    } else {
      shown.push({ kind: 'list', items: b.items.slice(0, room) });
      used += room;
      folded = true;
    }
  }
  return { shown, total, folded };
}
